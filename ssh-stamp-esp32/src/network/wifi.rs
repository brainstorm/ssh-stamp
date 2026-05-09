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
use esp_radio::Controller;
use esp_radio::wifi::{
    AccessPointConfig, AuthMethod, Config as RadioConfig, ModeConfig, WifiApState, WifiController,
    WifiDevice, WifiEvent,
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
    controller: Option<Controller<'static>>,
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
    pub fn new(
        spawner: Spawner,
        controller: Controller<'static>,
        wifi_peri: WIFI<'static>,
        rng: Rng,
        gateway: Ipv4Addr,
    ) -> Self {
        Self {
            spawner,
            controller: Some(controller),
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
        static CONTROLLER_CELL: StaticCell<Controller<'static>> = StaticCell::new();
        static RESOURCES_CELL: StaticCell<StackResources<3>> = StaticCell::new();
        static SSID_CELL: StaticCell<heapless::String<32>> = StaticCell::new();
        static PASSWORD_CELL: StaticCell<heapless::String<63>> = StaticCell::new();

        let ap_config = self
            .ap_config
            .clone()
            .ok_or(HalError::Wifi(WifiError::Initialization))?;
        let controller = self
            .controller
            .take()
            .ok_or(HalError::Wifi(WifiError::Initialization))?;
        let wifi_peri = self
            .wifi_peri
            .take()
            .ok_or(HalError::Wifi(WifiError::Initialization))?;

        // MAC must be set on eFuse before bringing up the radio.
        esp_hal::efuse::Efuse::set_mac_address(ap_config.mac)
            .map_err(|_| HalError::Wifi(WifiError::Initialization))?;

        let radio = &*CONTROLLER_CELL.init(controller);
        let (mut wifi_controller, interfaces) =
            esp_radio::wifi::new(radio, wifi_peri, RadioConfig::default())
                .map_err(|_| HalError::Wifi(WifiError::Initialization))?;

        let password = ap_config
            .password
            .as_ref()
            .map(|p| AllocString::from(p.as_str()))
            .unwrap_or_default();
        let ap_radio_config = ModeConfig::AccessPoint(
            AccessPointConfig::default()
                .with_ssid(AllocString::from(ap_config.ssid.as_str()))
                .with_auth_method(AuthMethod::Wpa2Wpa3Personal)
                .with_password(password.clone()),
        );
        let res = wifi_controller.set_config(&ap_radio_config);
        debug!("wifi_set_configuration returned {res:?}");

        let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
            address: Ipv4Cidr::new(self.gateway, 24),
            gateway: Some(self.gateway),
            dns_servers: heapless::Vec::new(),
        });

        let seed = u64::from(self.rng.random()) << 32 | u64::from(self.rng.random());

        let (ap_stack, runner) = embassy_net::new(
            interfaces.ap,
            net_config,
            RESOURCES_CELL.init(StackResources::<3>::new()),
            seed,
        );

        let ssid_static: &'static str = SSID_CELL.init(ap_config.ssid.clone()).as_str();
        let password_static: &'static str = PASSWORD_CELL
            .init(ap_config.password.clone().unwrap_or_default())
            .as_str();

        self.spawner
            .spawn(wifi_up(wifi_controller, ssid_static, password_static))
            .map_err(|_| HalError::Wifi(WifiError::Initialization))?;
        self.spawner
            .spawn(net_up(runner))
            .map_err(|_| HalError::Wifi(WifiError::Initialization))?;
        self.spawner
            .spawn(dhcp_server(ap_stack, self.gateway))
            .map_err(|_| HalError::Wifi(WifiError::Initialization))?;

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
pub async fn wifi_up(
    mut wifi_controller: WifiController<'static>,
    ssid: &'static str,
    password: &'static str,
) {
    debug!("Device capabilities: {:?}", wifi_controller.capabilities());

    loop {
        let client_config = ModeConfig::AccessPoint(
            AccessPointConfig::default()
                .with_ssid(AllocString::from(ssid))
                .with_auth_method(AuthMethod::Wpa2Wpa3Personal)
                .with_password(AllocString::from(password)),
        );

        if esp_radio::wifi::ap_state() == WifiApState::Started {
            wifi_controller.wait_for_event(WifiEvent::ApStop).await;
            Timer::after(Duration::from_millis(5000)).await;
        }
        if !matches!(wifi_controller.is_started(), Ok(true)) {
            if let Err(e) = wifi_controller.set_config(&client_config) {
                debug!("Failed to set wifi config: {e:?}");
                Timer::after(Duration::from_millis(1000)).await;
                continue;
            }
            debug!("Starting wifi");
            if let Err(e) = wifi_controller.start_async().await {
                debug!("Failed to start wifi: {e:?}");
                Timer::after(Duration::from_millis(1000)).await;
                continue;
            }
            debug!("Wifi started!");
        }
        Timer::after(Duration::from_millis(10)).await;
    }
}

/// Network task for Embassy executor.
#[embassy_executor::task]
pub async fn net_up(mut runner: Runner<'static, WifiDevice<'static>>) {
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
