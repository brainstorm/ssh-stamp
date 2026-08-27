// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The radio, presented to ssh-stamp as an `embassy_net::Stack`.
//!
//! `bl616-wifi` hands out an `embassy_net_driver::Driver` over the vendor MAC,
//! so this is mostly plumbing: configure, associate, build the stack, spawn
//! its runner. What is not plumbing is written down below.

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

/// The stack's socket storage. `'static` because the stack and its runner
/// outlive `bring_up`.
static RESOURCES: StaticCell<StackResources<SOCKETS>> = StaticCell::new();

/// The BL616 radio behind the HAL traits.
pub struct Bl616Wifi {
    wifi: Wifi,
    spawner: Spawner,
    config: Option<WifiApConfigStatic>,
}

impl Bl616Wifi {
    /// Take the radio. The vendor stack is already initialised by
    /// `bl616_wifi::main!`, which runs before any of this.
    ///
    /// # Errors
    ///
    /// Returns [`HalError::Wifi`] if the vendor manager will not start.
    pub fn new(spawner: Spawner) -> Result<Self, HalError> {
        let wifi = Wifi::init().map_err(|_| HalError::Wifi(WifiError::Initialization))?;
        Ok(Self {
            wifi,
            spawner,
            config: None,
        })
    }

    /// The station interface's MAC, which ssh-stamp uses to name the network.
    #[must_use]
    pub fn mac(&self) -> [u8; 6] {
        self.wifi.sta_mac()
    }
}

impl WifiHal for Bl616Wifi {
    fn configure_ap(&mut self, config: WifiApConfigStatic) -> Result<(), HalError> {
        // 2.4 GHz only. Accepting a 5 GHz request and quietly running at 2.4
        // would leave a user wondering why their band setting does nothing.
        if matches!(config.band, BandMode::Band5G) {
            return Err(HalError::Config);
        }
        self.config = Some(config);
        Ok(())
    }
}

impl NetworkProviderHal for Bl616Wifi {
    async fn bring_up(&mut self) -> Result<Stack<'static>, HalError> {
        let cfg = self.config.as_ref().ok_or(HalError::Config)?;

        // Station mode when an SSID has been stored, access point otherwise --
        // the same rule the ESP port uses, so behaviour does not depend on
        // which radio is underneath.
        let station = !cfg.sta_ssid.is_empty();

        let net_config = if station {
            self.wifi
                .connect(&StaConfig::wpa2(&cfg.sta_ssid, &cfg.sta_password))
                .map_err(|_| HalError::Wifi(WifiError::StationMode))?;
            Config::dhcpv4(embassy_net::DhcpConfig::default())
        } else {
            let ap = ApConfig::wpa2(&cfg.ap_ssid, &cfg.ap_password).on_channel(cfg.channel);
            let address = ap.address;
            let netmask = ap.netmask;
            self.wifi
                .start_ap(&ap)
                .map_err(|_| HalError::Wifi(WifiError::Initialization))?;

            let octets = address.as_raw().to_le_bytes();
            let cidr = Ipv4Cidr::new(
                Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]),
                // A prefix length is 0..=32, so this cannot truncate.
                u8::try_from(netmask.prefix_len()).unwrap_or(24),
            );
            Config::ipv4_static(StaticConfigV4 {
                address: cidr,
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

/// Drive the stack. Without this polling, nothing moves.
///
/// Monomorphic on `WifiDriver` because `#[embassy_executor::task]` cannot be
/// generic.
#[embassy_executor::task]
pub async fn net_up(mut runner: Runner<'static, WifiDriver>) -> ! {
    runner.run().await
}
