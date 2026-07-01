// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `WiFi` implementation for ESP32 family.
//!
//! Wraps `esp-radio` AP-mode `WiFi` behind the generic [`NetworkProviderHal`]
//! and [`WifiHal`] traits so the app layer never names ESP-specific types.

use core::net::Ipv4Addr;
use core::net::SocketAddrV4;

use alloc::string::String as AllocString;

use edge_dhcp::io::{self, DEFAULT_SERVER_PORT};
use edge_dhcp::server::{Server, ServerOptions};
use edge_nal::UdpBind;
use edge_nal_embassy::{Udp, UdpBuffers};
use embassy_executor::Spawner;
use embassy_net::DhcpConfig;
use embassy_net::tcp::TcpSocket;
use embassy_net::{IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::wifi::{
    AuthenticationMethod, BandMode as RadioBandMode, Config as RadioConfig, ControllerConfig,
    Interface, WifiController, ap::AccessPointConfig, ap::EventInfo, sta::StationConfig,
};
use log::info;
use log::{debug, error, warn};
use ssh_stamp::settings::STATION_MODE_MAX_RETRY_SECONDS;
use ssh_stamp_hal::{
    BandMode, HalError, NetworkProviderHal, WifiApConfigStatic, WifiError, WifiHal,
};
use static_cell::StaticCell;

extern crate alloc;

/// Handle for bringing up ESP32-family `WiFi` as an access point.
///
/// Construct with [`EspWifi::new`] once all ESP peripherals are available,
/// call [`WifiHal::configure_ap`] with the desired SSID/PSK/MAC, then call
/// [`NetworkProviderHal::bring_up`] to start the radio, spawn the driver
/// tasks, and return a ready [`embassy_net::Stack`].
pub struct EspWifi {
    spawner: Spawner,
    wifi_peri: Option<WIFI<'static>>,
    rng: Rng,
    ap_config: Option<WifiApConfigStatic>,
    gateway: Ipv4Addr,
}

impl EspWifi {
    /// Create a new uninitialised ESP32 `WiFi` handle.
    ///
    /// `gateway` is the static IPv4 address the device will serve as the
    /// access-point gateway (and DHCP server).
    #[must_use]
    pub fn new(spawner: Spawner, wifi_peri: WIFI<'static>, rng: Rng, gateway: Ipv4Addr) -> Self {
        Self {
            spawner,
            wifi_peri: Some(wifi_peri),
            rng,
            ap_config: None,
            gateway,
        }
    }
}

impl WifiHal for EspWifi {
    fn configure_ap(&mut self, config: WifiApConfigStatic) -> Result<(), HalError> {
        self.ap_config = Some(config);
        Ok(())
    }
}

impl NetworkProviderHal for EspWifi {
    async fn bring_up(&mut self) -> Result<Stack<'static>, HalError> {
        static RESOURCES_CELL: StaticCell<StackResources<3>> = StaticCell::new();
        static STA_SSID_CELL: StaticCell<heapless::String<32>> = StaticCell::new();

        let ap_config = self
            .ap_config
            .clone()
            .ok_or(HalError::Wifi(WifiError::Initialization))?;
        let wifi_peri = self
            .wifi_peri
            .take()
            .ok_or(HalError::Wifi(WifiError::Initialization))?;

        // MAC must be set on eFuse before bringing up the radio.
        esp_hal::efuse::override_mac_address(esp_hal::efuse::MacAddress::new_eui48(ap_config.mac))
            .map_err(|_| HalError::Wifi(WifiError::Initialization))?;

        let sta_ssid_static: &'static str = STA_SSID_CELL.init(ap_config.sta_ssid.clone()).as_str();
        let (ap_radio_config, net_config, wifi_interface) =
            build_radio_config(&ap_config, sta_ssid_static, self.gateway);

        let controller_config = ControllerConfig::default().with_initial_config(ap_radio_config);
        let mut wifi_controller = WifiController::new(wifi_peri, controller_config)
            .map_err(|_| HalError::Wifi(WifiError::Initialization))?;

        // Set the WiFi band mode (AP mode only). Only the ESP32-C5 supports
        // 5GHz; other chips ignore the setting.
        if sta_ssid_static.is_empty() {
            set_band_mode(&mut wifi_controller, ap_config.band);
        }

        let seed = u64::from(self.rng.random()) << 32 | u64::from(self.rng.random());

        let (ap_stack, runner) = embassy_net::new(
            wifi_interface,
            net_config,
            RESOURCES_CELL.init(StackResources::<3>::new()),
            seed,
        );

        self.spawner.spawn(
            wifi_up(wifi_controller, sta_ssid_static)
                .map_err(|_| HalError::Wifi(WifiError::Initialization))?,
        );
        self.spawner
            .spawn(net_up(runner).map_err(|_| HalError::Wifi(WifiError::Initialization))?);

        if sta_ssid_static.is_empty() {
            self.spawner.spawn(
                dhcp_server(ap_stack, self.gateway)
                    .map_err(|_| HalError::Wifi(WifiError::Initialization))?,
            );
            loop {
                debug!("Checking if link is up");
                if ap_stack.is_link_up() {
                    if let Some(config) = ap_stack.config_v4() {
                        info!(
                            "Connect to the AP `{}` with IP {}",
                            ap_config.ap_ssid.as_str(),
                            config.address,
                        );
                    }
                    break;
                }
                Timer::after(Duration::from_millis(500)).await;
            }
        } else {
            let mut retry_count = 0;
            loop {
                debug!("Checking if station has received IP address");
                if ap_stack.is_config_up() {
                    if let Some(config) = ap_stack.config_v4() {
                        info!(
                            "Connect to the AP `{}` with IP {}",
                            sta_ssid_static, config.address,
                        );
                    }
                    break;
                }
                retry_count += 1;
                if retry_count > STATION_MODE_MAX_RETRY_SECONDS {
                    return Err(HalError::Wifi(WifiError::StationMode));
                }
                Timer::after(Duration::from_millis(1000)).await;
            }
        }

        Ok(ap_stack)
    }
}

/// Build the esp-radio config, embassy-net config, and interface for AP or
/// Station mode based on whether a Station SSID is configured.
fn build_radio_config(
    ap_config: &WifiApConfigStatic,
    sta_ssid: &str,
    gateway: Ipv4Addr,
) -> (RadioConfig, embassy_net::Config, Interface) {
    if sta_ssid.is_empty() {
        info!("Wifi configuring Access Point Mode");
        let password = AllocString::from(ap_config.ap_password.as_str());
        let radio = RadioConfig::AccessPoint(
            AccessPointConfig::default()
                .with_ssid(AllocString::from(ap_config.ap_ssid.as_str()))
                .with_auth_method(AuthenticationMethod::Wpa2Wpa3Personal)
                .with_password(password)
                .with_channel(ap_config.channel),
        );
        let net = embassy_net::Config::ipv4_static(StaticConfigV4 {
            address: Ipv4Cidr::new(gateway, 24),
            gateway: Some(gateway),
            dns_servers: Default::default(),
        });
        (radio, net, Interface::access_point())
    } else {
        info!("Wifi configuring Station Mode");
        let password = AllocString::from(ap_config.sta_password.as_str());
        let radio = RadioConfig::Station(
            StationConfig::default()
                .with_ssid(AllocString::from(ap_config.sta_ssid.as_str()))
                .with_password(password),
        );
        let net = embassy_net::Config::dhcpv4(DhcpConfig::default());
        (radio, net, Interface::station())
    }
}

/// Set the `WiFi` band mode on the controller. Only the ESP32-C5 supports 5GHz;
/// on other chips `set_band_mode` returns an error that is logged and ignored.
fn set_band_mode(wifi_controller: &mut WifiController<'static>, band: BandMode) {
    let radio_band = match band {
        BandMode::Band2_4G => RadioBandMode::_2_4G,
        // _5G and Auto only exist when wifi_has_5g cfg is set (ESP32-C5).
        // On other chips, fall back to 2.4GHz.
        #[cfg(wifi_has_5g)]
        BandMode::Band5G => RadioBandMode::_5G,
        #[cfg(wifi_has_5g)]
        BandMode::Auto => RadioBandMode::Auto,
        #[cfg(not(wifi_has_5g))]
        _ => RadioBandMode::_2_4G,
    };
    match wifi_controller.set_band_mode(radio_band.clone()) {
        Ok(()) => debug!("Set WiFi band mode: {radio_band:?}"),
        Err(e) => warn!("Failed to set band mode {radio_band:?}: {e:?} (non-5G chip?)"),
    }
}

/// Accept an incoming TCP connection on port 22.
/// Returns a connected `TcpSocket` ready for SSH processing.
///
/// # Errors
/// Returns an error if the socket cannot be accepted.
/// Note that this function will block until a connection is accepted, and will
/// only return an error if there is a failure in the underlying socket operations.
pub async fn accept_requests<'a>(
    tcp_stack: Stack<'a>,
    rx_buffer: &'a mut [u8],
    tx_buffer: &'a mut [u8],
) -> Result<TcpSocket<'a>, HalError> {
    let mut tcp_socket = TcpSocket::new(tcp_stack, rx_buffer, tx_buffer);

    debug!("Waiting for SSH client...");
    if let Err(_e) = tcp_socket
        .accept(IpListenEndpoint {
            addr: None,
            port: 22,
        })
        .await
    {
        error!("Failed to accept incoming TCP connection");
        return Err(HalError::Wifi(WifiError::SocketAccept));
    }
    debug!("Connected, port 22");

    Ok(tcp_socket)
}

/// Manages the `WiFi` access point lifecycle.
#[embassy_executor::task]
pub async fn wifi_up(mut wifi_controller: WifiController<'static>, sta_ssid: &'static str) {
    // The controller keeps the radio alive.
    if sta_ssid.is_empty() {
        // Access Point Mode
        debug!("Wifi AP starting...");
        // If the radio ever goes down (e.g. hardware fault), esp-radio
        // currently has no public event API to detect it.
        loop {
            let ev = wifi_controller
                .wait_for_access_point_connected_event_async()
                .await;
            match ev {
                Ok(EventInfo::Connected(info)) => {
                    info!("Station connected: {info:?}");
                }
                Ok(EventInfo::Disconnected(info)) => {
                    info!("Station disconnected: {info:?}");
                }
                _ => (),
            }
            Timer::after(Duration::from_millis(5000)).await;
        }
    } else {
        // Station Mode
        // If the connection is lost it will attempt to reconnect.
        loop {
            debug!("Connecting to access point...");

            match wifi_controller.connect_async().await {
                Ok(info) => {
                    info!("Wifi connected to {info:?}");

                    // Wait until we're no longer connected
                    let info = wifi_controller.wait_for_disconnect_async().await.ok();
                    info!("Disconnected: {info:?}");
                }
                Err(e) => {
                    info!("Failed to connect to wifi: {e:?}");
                }
            }
            Timer::after(Duration::from_millis(1000)).await;
        }
    }
}

/// Network task for Embassy executor.
#[embassy_executor::task]
pub async fn net_up(mut runner: Runner<'static, Interface>) {
    debug!("Bringing up network stack...");
    runner.run().await;
}

/// DHCP server task for Embassy executor.
#[embassy_executor::task]
pub async fn dhcp_server(stack: Stack<'static>, ip: Ipv4Addr) {
    let mut buf = [0u8; 1500];
    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];

    let buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
    let unbound_socket = Udp::new(stack, &buffers);
    let mut bound_socket = match unbound_socket
        .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
    {
        Ok(socket) => socket,
        Err(e) => {
            warn!("Failed to bind DHCP server socket: {e:?}");
            return;
        }
    };

    loop {
        if let Err(e) = io::server::run(
            &mut Server::<_, 64>::new_with_et(ip),
            &ServerOptions::new(ip, Some(&mut gw_buf)),
            &mut bound_socket,
            &mut buf,
        )
        .await
        {
            error!("DHCP server error: {e:?}");
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}
