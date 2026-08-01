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
use embassy_net::{Config as NetConfig, Stack, StackResources};
use embassy_net_wiznet::chip::W6300;
use embassy_net_wiznet::{Device, Runner, State};
use embassy_rp::gpio::{Input, Output};
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio_programs::spi::Spi as PioSpi;
use embassy_rp::spi::{Async, Config as SpiConfig, Phase, Polarity};
use embassy_time::Delay;
use embedded_hal_bus::spi::ExclusiveDevice;
use log::{debug, info, warn};
use ssh_stamp_hal::{HalError, NetworkProviderHal, WifiError};
use static_cell::StaticCell;

/// SPI clock for the W6300. Conservative for first bring-up; the part is
/// rated far higher on single SPI, so raise it once a link is confirmed.
pub const W6300_SPI_CLK_HZ: u32 = 8_000_000;

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

        debug!("W6300: waiting for link");
        stack.wait_link_up().await;
        info!("W6300: link up, requesting DHCP lease");
        stack.wait_config_up().await;

        if let Some(cfg) = stack.config_v4() {
            info!("W6300: IPv4 {} (ssh to port 22)", cfg.address);
        }

        Ok(stack)
    }
}
