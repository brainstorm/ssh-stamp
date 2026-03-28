// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! WiFi implementation for ESP32 family
//!
//! Provides WiFi access point functionality for SSH-Stamp.

use core::net::Ipv4Addr;
use core::net::SocketAddrV4;

use edge_dhcp::io::{self, DEFAULT_SERVER_PORT};
use edge_dhcp::server::{Server, ServerOptions};
use edge_nal::UdpBind;
use edge_nal_embassy::{Udp, UdpBuffers};
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_hal::efuse::Efuse;
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::wifi::{
    AccessPointConfig, AuthMethod, ModeConfig, WifiApState, WifiController, WifiEvent,
};
use esp_radio::Controller;
use hal::{HalError, WifiApConfigStatic, WifiError, WifiHal};
use heapless::String;
use log::{debug, error, info, warn};

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String as AllocString;
use alloc::string::ToString;

/// WiFi password character set for generation
pub const WIFI_PASSWORD_CHARS: &[u8; 62] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Default WiFi SSID
pub const DEFAULT_SSID: &str = "ssh-stamp";

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

/// ESP32 WiFi implementation
pub struct EspWifi {
    ap_config: Option<WifiApConfigStatic>,
}

impl EspWifi {
    pub fn new() -> Self {
        Self { ap_config: None }
    }
}

impl Default for EspWifi {
    fn default() -> Self {
        Self::new()
    }
}

impl WifiHal for EspWifi {
    async fn start_ap(&mut self, config: WifiApConfigStatic) -> Result<(), HalError> {
        self.ap_config = Some(config);
        Ok(())
    }
}

/// WiFi task for Embassy executor
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
                debug!("Failed to set wifi config: {:?}", e);
                Timer::after(Duration::from_millis(1000)).await;
                continue;
            }
            debug!("Starting wifi");
            if let Err(e) = wifi_controller.start_async().await {
                debug!("Failed to start wifi: {:?}", e);
                Timer::after(Duration::from_millis(1000)).await;
                continue;
            }
            debug!("Wifi started!");
        }
        Timer::after(Duration::from_millis(10)).await;
    }
}

/// Network task for Embassy executor
#[embassy_executor::task]
pub async fn net_up(mut runner: Runner<'static, esp_radio::wifi::WifiDevice<'static>>) {
    debug!("Bringing up network stack...");
    runner.run().await
}

/// DHCP server task for Embassy executor
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

/// Accept incoming TCP connection
pub async fn accept_requests<'a>(
    tcp_stack: Stack<'a>,
    rx_buffer: &'a mut [u8],
    tx_buffer: &'a mut [u8],
) -> TcpSocket<'a> {
    let mut tcp_socket = TcpSocket::new(tcp_stack, rx_buffer, tx_buffer);

    debug!("Waiting for SSH client...");
    if let Err(_e) = tcp_socket
        .accept(IpListenEndpoint {
            addr: None,
            port: 22,
        })
        .await
    {
        // Continue trying to accept
    }
    debug!("Connected, port 22");

    tcp_socket
}

/// Initialize WiFi AP with given configuration
#[allow(dead_code)]
pub async fn init_wifi_ap(
    spawner: Spawner,
    controller: Controller<'static>,
    wifi: WIFI<'static>,
    rng: Rng,
    ssid: String<32>,
    password: String<63>,
    mac: [u8; 6],
    gw_ip: Ipv4Addr,
) -> Result<Stack<'static>, HalError> {
    let wifi_init = &*mk_static!(Controller<'static>, controller);
    let (mut wifi_controller, interfaces) =
        esp_radio::wifi::new(wifi_init, wifi, Default::default())
            .map_err(|_| HalError::Wifi(WifiError::Initialization))?;

    // Set MAC address
    Efuse::set_mac_address(mac).map_err(|_| HalError::Config)?;

    let ap_config = ModeConfig::AccessPoint(
        AccessPointConfig::default()
            .with_ssid(AllocString::from(ssid.as_str()))
            .with_auth_method(AuthMethod::Wpa2Wpa3Personal)
            .with_password(AllocString::from(password.as_str())),
    );
    let _res = wifi_controller.set_config(&ap_config);

    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip, 24),
        gateway: Some(gw_ip),
        dns_servers: Default::default(),
    });

    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (ap_stack, runner) = embassy_net::new(
        interfaces.ap,
        net_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    // Convert ssid and password to static - caller must ensure they live long enough
    // For now we use leak to make them 'static
    let ssid_static: &'static str = Box::leak(ssid.as_str().to_string().into_boxed_str());
    let password_static: &'static str = Box::leak(password.as_str().to_string().into_boxed_str());

    spawner
        .spawn(wifi_up(wifi_controller, ssid_static, password_static))
        .ok();
    spawner.spawn(net_up(runner)).ok();
    spawner.spawn(dhcp_server(ap_stack, gw_ip)).ok();

    loop {
        debug!("Checking if link is up");
        if ap_stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    info!(
        "Connect to the AP `ssh-stamp` as a DHCP client with IP: {}",
        gw_ip
    );

    Ok(ap_stack)
}

/// Disable AP stack
pub async fn ap_stack_disable() {
    debug!("AP Stack disabled: WIP");
}

/// Disable TCP socket
pub async fn tcp_socket_disable() {
    debug!("TCP socket disabled: WIP");
}

/// Disable WiFi controller
pub async fn wifi_controller_disable() {
    debug!("Disabling wifi: WIP");
}
