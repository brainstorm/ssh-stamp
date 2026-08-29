// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The radio, presented to ssh-stamp as an `embassy_net::Stack`.
//!
//! `bl616-wifi` hands out an `embassy_net_driver::Driver` over the vendor MAC,
//! so this is mostly plumbing: configure, associate, build the stack, spawn
//! its runner. What is not plumbing is written down below.

use bl616_dhcp::{CLIENT_PORT, Leases, SERVER_PORT};
use bl616_wifi::net_al::embassy::WifiDriver;
use bl616_wifi::{ApConfig, StaConfig, Wifi};
use embassy_executor::Spawner;
use embassy_net::{Config, Ipv4Address, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use ssh_stamp_hal::{
    BandMode, HalError, NetworkProviderHal, WifiApConfigStatic, WifiError, WifiHal,
};
use static_cell::StaticCell;

/// Sockets the stack may have open at once: SSH, plus room for a DHCP server
/// and a spare.
const SOCKETS: usize = 4;

/// First host number handed out by the soft-AP, and how many.
const DHCP_POOL_START: u16 = 2;
const DHCP_POOL_LIMIT: u16 = 16;

/// The stack's socket storage. `'static` because the stack and its runner
/// outlive `bring_up`.
static RESOURCES: StaticCell<StackResources<SOCKETS>> = StaticCell::new();

/// The BL616 radio behind the HAL traits.
pub struct Bl616Wifi {
    wifi: Wifi,
    spawner: Spawner,
    config: Option<WifiApConfigStatic>,
    /// The soft-AP's address, recorded by `start` so the async half does not
    /// have to ask the vendor for it.
    address: Option<(u32, u32)>,
    started: bool,
}

impl Bl616Wifi {
    /// Wait for the vendor stack to finish coming up.
    ///
    /// **Call this before starting the embassy executor, not from inside a
    /// task.** It blocks on `FreeRTOS` primitives while the radio initialises,
    /// and doing that from within `executor.poll()` hangs: the board reaches
    /// this call and never leaves it. `bl616-wifi`'s own embassy examples do
    /// the same thing in the same order, which is the arrangement known to
    /// work.
    ///
    /// # Errors
    ///
    /// Returns [`HalError::Wifi`] if the vendor manager will not start.
    pub fn init_radio() -> Result<Wifi, HalError> {
        Wifi::init().map_err(|_| HalError::Wifi(WifiError::Initialization))
    }

    /// Wrap an initialised radio, once there is an executor to spawn on.
    #[must_use]
    pub fn new(wifi: Wifi, spawner: Spawner) -> Self {
        Self {
            wifi,
            spawner,
            config: None,
            address: None,
            started: false,
        }
    }

    /// The station interface's MAC, which ssh-stamp uses to name the network.
    #[must_use]
    pub fn mac(&self) -> [u8; 6] {
        self.wifi.sta_mac()
    }
}

impl WifiHal for Bl616Wifi {
    fn configure_ap(&mut self, config: WifiApConfigStatic) -> Result<(), HalError> {
        // The BL616 has a 2.4 GHz radio and no 5 GHz one: this is the part,
        // not the port, so there is nothing here to implement later.
        // Accepting the request and quietly running at 2.4 would leave a
        // user wondering why their band setting does nothing, so it is
        // refused — with a reason, since `HalError::Config` on its own does
        // not tell anyone which setting was wrong.
        if matches!(config.band, BandMode::Band5G) {
            bl616_wifi::println!(
                "[ssh-stamp] 5 GHz was requested, but the BL616 radio is 2.4 GHz only; \
                 set SSH_STAMP_WIFI_BAND to 2.4"
            );
            return Err(HalError::Config);
        }
        self.config = Some(config);
        Ok(())
    }
}

impl Bl616Wifi {
    /// Start the radio in AP or station mode.
    ///
    /// **Call this before the executor exists.** Every vendor call here
    /// blocks on `FreeRTOS` primitives, and doing that from inside
    /// `executor.poll()` hangs the board with no timeout -- the same reason
    /// [`Bl616Wifi::init_radio`] is separate.
    ///
    /// # Errors
    ///
    /// [`HalError::Config`] if no configuration was set, or
    /// [`HalError::Wifi`] if the vendor stack refuses to start.
    pub fn start(&mut self) -> Result<(), HalError> {
        let cfg = self.config.clone().ok_or(HalError::Config)?;
        let (addr, prefix) = Self::start_radio(&self.wifi, &cfg)?;
        self.address = Some((addr, prefix));
        self.started = true;
        Ok(())
    }

    /// Start the radio from a bare [`Wifi`] handle, before any executor
    /// exists. Returns the soft-AP's address and prefix length.
    ///
    /// This is the half that must not run inside the embassy executor. Every
    /// call it makes blocks on `FreeRTOS` primitives, and doing that once an
    /// executor owns the task hangs the board with no timeout.
    ///
    /// # Errors
    ///
    /// [`HalError::Wifi`] if the vendor stack refuses to start.
    pub fn start_radio(wifi: &Wifi, cfg: &WifiApConfigStatic) -> Result<(u32, u32), HalError> {
        if cfg.sta_ssid.is_empty() {
            let ap = ApConfig::wpa2(&cfg.ap_ssid, &cfg.ap_password).on_channel(cfg.channel);
            let addr = (ap.address.as_raw(), ap.netmask.prefix_len());
            wifi.start_ap(&ap)
                .map_err(|_| HalError::Wifi(WifiError::Initialization))?;
            Ok(addr)
        } else {
            wifi.connect(&StaConfig::wpa2(&cfg.sta_ssid, &cfg.sta_password))
                .map_err(|_| HalError::Wifi(WifiError::StationMode))?;
            Ok((0, 0))
        }
    }

    /// Adopt a radio that [`Bl616Wifi::start_radio`] has already started.
    pub fn adopt(&mut self, config: WifiApConfigStatic, address: (u32, u32)) {
        self.config = Some(config);
        self.address = Some(address);
        self.started = true;
    }
}

impl NetworkProviderHal for Bl616Wifi {
    async fn bring_up(&mut self) -> Result<Stack<'static>, HalError> {
        if !self.started {
            return Err(HalError::Config);
        }
        let cfg = self.config.as_ref().ok_or(HalError::Config)?;

        // Station mode when an SSID has been stored, access point otherwise --
        // the same rule the ESP port uses, so behaviour does not depend on
        // which radio is underneath.
        let station = !cfg.sta_ssid.is_empty();

        let net_config = if station {
            Config::dhcpv4(embassy_net::DhcpConfig::default())
        } else {
            let (addr, prefix) = self.address.ok_or(HalError::Config)?;
            let octets = addr.to_le_bytes();
            Config::ipv4_static(StaticConfigV4 {
                address: Ipv4Cidr::new(
                    Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]),
                    u8::try_from(prefix).unwrap_or(24),
                ),
                // An access point is the edge of its own network: there is
                // nothing to forward to.
                gateway: None,
                dns_servers: Default::default(),
            })
        };

        // The blob registers the station first and the soft-AP second, so the
        // AP is interface 1. Binding blindly to 0 serves the wrong interface,
        // which this project has already got wrong once.
        let index = usize::from(!station);
        let driver = loop {
            if let Some(d) = WifiDriver::new(index) {
                break d;
            }
            embassy_time::Timer::after_millis(50).await;
        };

        // Randomise port and sequence-number choice from the hardware TRNG
        // rather than from the clock: this seeds a stack that will carry SSH.
        let seed = crate::rng::u64().map_err(|_| HalError::Rng)?;

        let (stack, runner) = embassy_net::new(
            driver,
            net_config,
            RESOURCES.init(StackResources::new()),
            seed,
        );

        self.spawner
            .spawn(net_up(runner).map_err(|_| HalError::Wifi(WifiError::Initialization))?);

        // An access point has to answer DHCP or nothing that joins it can
        // reach the SSH port.
        if !station {
            let (addr, prefix) = self.address.ok_or(HalError::Config)?;
            let mask = (!0u32) << (32 - prefix);
            self.spawner.spawn(
                dhcp_server(
                    stack,
                    addr,
                    u32::from_le_bytes(mask.to_be_bytes()),
                    DHCP_POOL_START,
                    DHCP_POOL_LIMIT,
                )
                .map_err(|_| HalError::Wifi(WifiError::Initialization))?,
            );
        }

        stack.wait_config_up().await;

        // Hand the address back to the blob. Nothing else writes those fields
        // when the stack lives here, so `wifi_sta_ip4_addr_get` and the vendor
        // CLI would otherwise report no address at all.
        if let Some(v4) = stack.config_v4() {
            let addr = v4.address.address().octets();
            let mask = (!0u32) << (32 - v4.address.prefix_len());
            let gw = v4.gateway.map_or([0; 4], |g| g.octets());
            let dns = v4
                .dns_servers
                .first()
                .map_or([0; 4], embassy_net::Ipv4Address::octets);
            bl616_wifi::net_al::set_vif_addr(
                index,
                u32::from_le_bytes(addr),
                u32::from_le_bytes(mask.to_be_bytes()),
                u32::from_le_bytes(gw),
                u32::from_le_bytes(dns),
            );
        }

        Ok(stack)
    }
}

/// Serve DHCP to whoever joins the soft-AP.
///
/// embassy-net has a DHCP *client* and no server, so an access point that
/// does not run one hands out no addresses: a client associates, waits, and
/// gives up with "IP configuration could not be reserved". `bl616-dhcp` has
/// the protocol; this is the socket around it.
#[embassy_executor::task]
pub async fn dhcp_server(stack: Stack<'static>, server: u32, mask: u32, start: u16, limit: u16) {
    use embassy_net::IpEndpoint;
    use embassy_net::udp::{PacketMetadata, UdpSocket};

    static RX_META: StaticCell<[PacketMetadata; 8]> = StaticCell::new();
    static TX_META: StaticCell<[PacketMetadata; 8]> = StaticCell::new();
    static RX_BUF: StaticCell<[u8; 1500]> = StaticCell::new();
    static TX_BUF: StaticCell<[u8; 1500]> = StaticCell::new();

    let Some(mut leases) = Leases::new(server, mask, start, limit) else {
        return;
    };
    let mut sock = UdpSocket::new(
        stack,
        RX_META.init([PacketMetadata::EMPTY; 8]),
        RX_BUF.init([0; 1500]),
        TX_META.init([PacketMetadata::EMPTY; 8]),
        TX_BUF.init([0; 1500]),
    );
    if sock.bind(SERVER_PORT).is_err() {
        return;
    }

    let mut req = [0u8; 1024];
    let mut reply = [0u8; 548];
    loop {
        let Ok((n, _from)) = sock.recv_from(&mut req).await else {
            continue;
        };
        let Some(len) = leases.handle(&req[..n], &mut reply) else {
            continue;
        };
        // Always broadcast: the client has no address yet, so a unicast reply
        // would need an ARP entry it cannot answer.
        let to = IpEndpoint::new(Ipv4Address::BROADCAST.into(), CLIENT_PORT);
        let _ = sock.send_to(&reply[..len], to).await;
    }
}

/// Drive the stack. Without this polling, nothing moves.
///
/// Monomorphic on `WifiDriver` because `#[embassy_executor::task]` cannot be
/// generic.
#[embassy_executor::task]
pub async fn net_up(mut runner: Runner<'static, WifiDriver>) -> ! {
    runner.run().await
}
