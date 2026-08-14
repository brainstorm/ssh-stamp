// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Wired Ethernet via the `WIZnet` W6300, satisfying
//! [`ssh_stamp_hal::NetworkProviderHal`].
//!
//! # Why PIO instead of the hardware SPI block
//!
//! The W6300-EVB-Pico2 wires `(CS, SCK, IO0, IO1) = (GPIO16, 17, 18, 19)`.
//! The RP2350's SPI0 function map wants `(16, 17, 18, 19) = (RX, CSn, SCK,
//! TX)` — SCK and the data lines do not line up, so the hardware block
//! cannot drive this board without rewiring. A PIO state machine can put
//! SPI on arbitrary pins, so that is what we use (`embassy_rp`'s stock
//! `pio_programs::spi`).
//!
//! # Why single SPI, not QSPI
//!
//! The board's selling point is quad SPI (IO2/IO3 on GPIO20/21, >80 Mbps).
//! `embassy-net-wiznet` 0.3 is single-SPI only for the W6300 — quad support
//! is embassy-rs/embassy#4662, still open. IO2/IO3 are therefore left
//! unconfigured and we run single-bit at [`W6300_SPI_CLK_HZ`]. This is a
//! throughput ceiling, not a correctness problem: an SSH terminal bridge
//! needs kilobits, not megabits.

use embassy_executor::Spawner;
use embassy_net::{
    ConfigV4, Config as NetConfig, Ipv4Address, Ipv4Cidr, StaticConfigV4, Stack, StackResources,
};
use embassy_net_wiznet::chip::W6300;
use embassy_net_wiznet::{Device, Runner, State};
use embassy_rp::gpio::{Input, Output};
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio_programs::spi::Spi as PioSpi;
use embassy_rp::spi::{Async, Config as SpiConfig, Phase, Polarity};
use embassy_time::{Delay, Duration, Timer, with_timeout};
use embedded_hal_async::spi::{Operation, SpiDevice};
use embedded_hal_bus::spi::ExclusiveDevice;
use log::{error, info, warn};
use ssh_stamp_hal::{HalError, NetworkProviderHal, WifiError};
use static_cell::StaticCell;

/// SPI clock for the W6300. Conservative for first bring-up; the part is
/// rated far higher on single SPI, so raise it once a link is confirmed.
pub const W6300_SPI_CLK_HZ: u32 = 8_000_000;

/// How long to wait for the PHY to report a link before giving up.
///
/// The link either comes up in a second or two or something is wrong; a
/// board with no cable should say so rather than hang forever.
const LINK_TIMEOUT: Duration = Duration::from_secs(20);

/// Address used when no DHCP server answers.
///
/// 192.168.4.1 to match what the ESP32 ports serve on their access point,
/// so the address to point a client at is the same story on every board.
///
/// A future refinement is to take this from `SSHStampConfig::ipv4_static`,
/// which already exists and is already persisted; it is a constant here
/// because `bring_up` has no view of the stored config.
pub const FALLBACK_IPV4: Ipv4Address = Ipv4Address::new(192, 168, 4, 1);
/// Prefix length for [`FALLBACK_IPV4`].
pub const FALLBACK_PREFIX: u8 = 24;

/// How long to wait for a DHCP lease.
///
/// Generous: some switches hold a port down for several seconds of
/// spanning-tree learning before forwarding the first DISCOVER.
const DHCP_TIMEOUT: Duration = Duration::from_secs(45);

/// Reads a W6300 register block directly, bypassing the driver.
///
/// `embassy-net-wiznet` seals its `Chip` trait, so the framing is repeated
/// here: block byte, big-endian address, one dummy byte, then the data
/// phase. Used only by [`probe_w6300`], before the driver takes the bus.
async fn raw_read<SPI: SpiDevice>(
    spi: &mut SPI,
    block: u8,
    addr: u16,
    data: &mut [u8],
) -> Result<(), SPI::Error> {
    let instruction = [block];
    let address = addr.to_be_bytes();
    let dummy = [0u8];
    spi.transaction(&mut [
        Operation::Write(&instruction),
        Operation::Write(&address),
        Operation::Write(&dummy),
        Operation::TransferInPlace(data),
    ])
    .await
}

/// Logs the chip's identity and PHY status before handing over the bus.
///
/// Two questions this answers when a board does not come up, which are
/// otherwise indistinguishable from each other:
///
/// - Does SPI work at all? `VERSION` reads `0x11` on a healthy W6300, and
///   `0x00`/`0xff` when MISO is dead or the clock is wrong.
/// - Does the PHY see a link? The driver decides this from bit 0 of
///   `PHYSR`, so the raw value is worth seeing next to what the host end of
///   the cable reports.
///
/// Read before the driver's own reset, so this reflects the state the chip
/// reached on its own since power-up.
async fn probe_w6300<SPI: SpiDevice>(spi: &mut SPI) {
    const COMMON: u8 = 0x00;
    const VERSION_ADDR: u16 = 0x0004;
    const PHYSR_ADDR: u16 = 0x3000;

    let mut version = [0u8];
    match raw_read(spi, COMMON, VERSION_ADDR, &mut version).await {
        Ok(()) => info!(
            "W6300 probe: VERSION=0x{:02x} (expect 0x11)",
            version[0]
        ),
        Err(_) => {
            warn!("W6300 probe: VERSION read failed (SPI error)");
            return;
        }
    }

    // Sampled rather than read once: the PHY may still be negotiating, and
    // a bit that never sets is the interesting case.
    for i in 0..5 {
        let mut physr = [0u8];
        match raw_read(spi, COMMON, PHYSR_ADDR, &mut physr).await {
            Ok(()) => info!(
                "W6300 probe: PHYSR=0x{:02x} (link bit0={}) [{}/5]",
                physr[0],
                physr[0] & 1,
                i + 1
            ),
            Err(_) => warn!("W6300 probe: PHYSR read failed (SPI error)"),
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}

/// Concrete SPI device handed to the `WIZnet` driver: the PIO bus plus a
/// manually toggled CS.
pub type W6300Spi = ExclusiveDevice<PioSpi<'static, PIO0, 0, Async>, Output<'static>, Delay>;

type W6300Runner = Runner<'static, W6300, W6300Spi, Input<'static>, Output<'static>>;

/// Socket buffers for the driver. Two each is plenty for a shell session
/// and keeps RAM use modest.
static ETH_STATE: StaticCell<State<2, 2>> = StaticCell::new();
static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

/// SPI config the W6300 expects: mode 0, MSB first.
#[must_use]
pub fn spi_config() -> SpiConfig {
    let mut cfg = SpiConfig::default();
    cfg.frequency = W6300_SPI_CLK_HZ;
    cfg.phase = Phase::CaptureOnFirstTransition;
    cfg.polarity = Polarity::IdleLow;
    cfg
}

/// Pumps the `WIZnet` driver: `MACRAW` frames in and out of the chip.
#[embassy_executor::task]
async fn ethernet_task(runner: W6300Runner) -> ! {
    runner.run().await
}

/// Pumps the embassy-net stack (ARP, DHCP, TCP timers).
#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, Device<'static>>) -> ! {
    runner.run().await
}

/// Wired Ethernet provider for the W6300-EVB-Pico2.
pub struct W6300Ethernet {
    spawner: Spawner,
    mac: [u8; 6],
    seed: u64,
    /// Taken by [`NetworkProviderHal::bring_up`]; `None` afterwards.
    parts: Option<(W6300Spi, Input<'static>, Output<'static>)>,
}

impl W6300Ethernet {
    /// `int` is the W6300's `INTn` (active low), `reset` its `RSTn` — the
    /// driver pulses reset itself during init.
    #[must_use]
    pub fn new(
        spawner: Spawner,
        mac: [u8; 6],
        seed: u64,
        spi: W6300Spi,
        int: Input<'static>,
        reset: Output<'static>,
    ) -> Self {
        Self {
            spawner,
            mac,
            seed,
            parts: Some((spi, int, reset)),
        }
    }
}

impl NetworkProviderHal for W6300Ethernet {
    async fn bring_up(&mut self) -> Result<Stack<'static>, HalError> {
        let (spi, int, reset) = self
            .parts
            .take()
            .ok_or(HalError::Wifi(WifiError::Initialization))?;

        info!(
            "W6300: MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}, SPI {} Hz",
            self.mac[0],
            self.mac[1],
            self.mac[2],
            self.mac[3],
            self.mac[4],
            self.mac[5],
            W6300_SPI_CLK_HZ
        );

        let mut spi = spi;
        probe_w6300(&mut spi).await;

        let state = ETH_STATE.init(State::<2, 2>::new());
        let (device, runner) =
            embassy_net_wiznet::new::<2, 2, W6300, _, _, _>(self.mac, state, spi, int, reset)
                .await
                .map_err(|e| {
                    // Almost always either the SPI wiring/clock or a chip
                    // that never left reset: the driver checks VERSIONR
                    // before anything else.
                    warn!("W6300 init failed ({e:?}); check SPI wiring, clock and RSTn");
                    HalError::Wifi(WifiError::Initialization)
                })?;

        self.spawner
            .spawn(ethernet_task(runner).map_err(|_| HalError::Wifi(WifiError::Initialization))?);

        // No AP fallback on this board: DHCP is the only way in.
        let (stack, net_runner) = embassy_net::new(
            device,
            NetConfig::dhcpv4(embassy_net::DhcpConfig::default()),
            RESOURCES.init(StackResources::<3>::new()),
            self.seed,
        );

        self.spawner
            .spawn(net_task(net_runner).map_err(|_| HalError::Wifi(WifiError::Initialization))?);

        // Bounded and logged rather than a bare await: these two calls
        // block forever on a board with no cable, an unsupported PHY bit or
        // no DHCP server, and a silent hang gives no clue which it was.
        info!("W6300: waiting for link (up to {}s)", LINK_TIMEOUT.as_secs());
        if with_timeout(LINK_TIMEOUT, stack.wait_link_up()).await.is_err() {
            error!(
                "W6300: no link after {}s. The driver reads bit 0 of PHYSR; \
                 compare the probe values above against what the other end \
                 of the cable reports.",
                LINK_TIMEOUT.as_secs()
            );
            return Err(HalError::Wifi(WifiError::Initialization));
        }

        info!(
            "W6300: link up, requesting DHCP lease (up to {}s)",
            DHCP_TIMEOUT.as_secs()
        );
        if with_timeout(DHCP_TIMEOUT, stack.wait_config_up())
            .await
            .is_err()
        {
            // Directly attached to a laptop, or on a segment with no DHCP
            // server, there is nothing to wait for. Take a fixed address so
            // the board is still reachable rather than sitting unusable.
            warn!(
                "W6300: no DHCP lease after {}s, falling back to static {}/{}",
                DHCP_TIMEOUT.as_secs(),
                FALLBACK_IPV4,
                FALLBACK_PREFIX
            );
            stack.set_config_v4(ConfigV4::Static(StaticConfigV4 {
                address: Ipv4Cidr::new(FALLBACK_IPV4, FALLBACK_PREFIX),
                // No router and no resolver: this is a point-to-point link
                // to whatever is on the other end of the cable.
                gateway: None,
                // `Default` rather than `heapless::Vec::new()`: embassy-net
                // builds against a different heapless major than this
                // workspace, so the type cannot be named here.
                dns_servers: Default::default(),
            }));

            // Applying a static config completes immediately, but wait so
            // the address is actually live before anything binds to it.
            if with_timeout(Duration::from_secs(5), stack.wait_config_up())
                .await
                .is_err()
            {
                error!("W6300: static fallback did not come up");
                return Err(HalError::Wifi(WifiError::Initialization));
            }
        }

        if let Some(cfg) = stack.config_v4() {
            info!("W6300: IPv4 {} (ssh to port 22)", cfg.address);
        }

        Ok(stack)
    }
}
