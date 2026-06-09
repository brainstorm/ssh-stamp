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
use embassy_net::tcp::TcpSocket;
use embassy_net::{IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::wifi::{
    AuthenticationMethod, Config as RadioConfig, ControllerConfig, Interface, WifiController,
    ap::AccessPointConfig,
};
use log::{debug, error, warn};
use ssh_stamp_hal::{HalError, NetworkProviderHal, WifiApConfigStatic, WifiError, WifiHal};
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
        static SSID_CELL: StaticCell<heapless::String<32>> = StaticCell::new();
        static PASSWORD_CELL: StaticCell<heapless::String<63>> = StaticCell::new();

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

        let password = AllocString::from(ap_config.password.as_str());
        let ap_radio_config = RadioConfig::AccessPoint(
            AccessPointConfig::default()
                .with_ssid(AllocString::from(ap_config.ssid.as_str()))
                .with_auth_method(AuthenticationMethod::Wpa2Wpa3Personal)
                .with_password(password.clone()),
        );

        let ap_interface = Interface::access_point();

        let controller_config = ControllerConfig::default().with_initial_config(ap_radio_config);
        let wifi_controller = WifiController::new(wifi_peri, controller_config)
            .map_err(|_| HalError::Wifi(WifiError::Initialization))?;

        let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
            address: Ipv4Cidr::new(self.gateway, 24),
            gateway: Some(self.gateway),
            dns_servers: Default::default(),
        });

        let seed = u64::from(self.rng.random()) << 32 | u64::from(self.rng.random());

        let (ap_stack, runner) = embassy_net::new(
            ap_interface,
            net_config,
            RESOURCES_CELL.init(StackResources::<3>::new()),
            seed,
        );

        let ssid_static: &'static str = SSID_CELL.init(ap_config.ssid.clone()).as_str();
        let password_static: &'static str = PASSWORD_CELL.init(ap_config.password.clone()).as_str();

        self.spawner.spawn(
            wifi_up(wifi_controller, ssid_static, password_static)
                .map_err(|_| HalError::Wifi(WifiError::Initialization))?,
        );
        self.spawner
            .spawn(net_up(runner).map_err(|_| HalError::Wifi(WifiError::Initialization))?);
        self.spawner.spawn(
            dhcp_server(ap_stack, self.gateway)
                .map_err(|_| HalError::Wifi(WifiError::Initialization))?,
        );

        loop {
            debug!("Checking if link is up");
            if ap_stack.is_link_up() {
                break;
            }
            Timer::after(Duration::from_millis(500)).await;
        }

        Ok(ap_stack)
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
///
/// In esp-radio 1.0+, `set_config` both configures and starts the radio.
/// We call it once and retry only on error, preserving client connections.
#[embassy_executor::task]
pub async fn wifi_up(
    mut wifi_controller: WifiController<'static>,
    ssid: &'static str,
    password: &'static str,
) {
    let ap_config = RadioConfig::AccessPoint(
        AccessPointConfig::default()
            .with_ssid(AllocString::from(ssid))
            .with_auth_method(AuthenticationMethod::Wpa2Wpa3Personal)
            .with_password(AllocString::from(password)),
    );

    loop {
        match wifi_controller.set_config(&ap_config) {
            Ok(()) => {
                debug!("Wifi started!");
                // AP is up — sleep forever. The controller keeps the radio alive.
                // If the radio ever goes down (e.g. hardware fault), esp-radio
                // currently has no public event API to detect it, so we just hold.
                core::future::pending::<()>().await;
            }
            Err(e) => {
                debug!("Failed to set wifi config: {e:?}");
                Timer::after(Duration::from_millis(1000)).await;
            }
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
