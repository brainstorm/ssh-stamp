// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use log::{debug, error, info};

use crate::config::SSHStampConfig;
use crate::settings::{DEFAULT_IP, DEFAULT_SSID};
use core::net::Ipv4Addr;
use core::net::SocketAddrV4;
use edge_dhcp;
use edge_dhcp::{
    io::{self, DEFAULT_SERVER_PORT},
    server::{Server, ServerOptions},
};
use edge_nal::UdpBind;
use edge_nal_embassy::{Udp, UdpBuffers};
use embassy_executor::Spawner;
use embassy_net::{IpListenEndpoint, Ipv4Cidr, Runner, StaticConfigV4};
use embassy_net::{Stack, StackResources, tcp::TcpSocket};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_hal::system::software_reset;
use esp_radio::Controller;
use esp_radio::wifi::WifiEvent;
use esp_radio::wifi::{AccessPointConfig, ModeConfig, WifiApState, WifiController};
use heapless::String;
use sunset_async::SunsetMutex;
// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

pub async fn if_up(
    spawner: Spawner,
    controller: Controller<'static>,
    wifi: WIFI<'static>,
    rng: Rng,
    config: &'static SunsetMutex<SSHStampConfig>,
) -> Result<Stack<'static>, sunset::Error> {
    let wifi_init = &*mk_static!(Controller<'static>, controller);
    let (mut wifi_controller, interfaces) =
        esp_radio::wifi::new(wifi_init, wifi, Default::default())
            .map_err(|_| sunset::error::BadUsage.build())?;

    let ap_config =
        ModeConfig::AccessPoint(AccessPointConfig::default().with_ssid(DEFAULT_SSID.into()));
    let res = wifi_controller.set_config(&ap_config);
    info!("wifi_set_configuration returned {:?}", res);

    let gw_ip_addr_ipv4 = *DEFAULT_IP;

    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip_addr_ipv4, 24),
        gateway: Some(gw_ip_addr_ipv4),
        dns_servers: Default::default(),
    });

    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let (ap_stack, runner) = embassy_net::new(
        interfaces.ap,
        net_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(wifi_up(wifi_controller, config)).ok();
    spawner.spawn(net_up(runner)).ok();
    spawner.spawn(dhcp_server(ap_stack, gw_ip_addr_ipv4)).ok();

    loop {
        log::debug!("Checking if link is up");
        if ap_stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    // TODO: Use wifi_manager instead?
    info!(
        "Connect to the AP `ssh-stamp` as a DHCP client with IP: {}",
        gw_ip_addr_ipv4
    );

    Ok(ap_stack)
}

pub async fn ap_stack_disable() -> () {
    // drop ap_stack
    info!("AP Stack disabled");
    // TODO: Correctly disable/restart AP Stack and/or send messsage to user over SSH
    software_reset();
}

pub async fn tcp_socket_disable() -> () {
    // drop tcp stack
    info!("TCP socket disabled");
    // TODO: Correctly disable/restart tcp socket and/or send messsage to user over SSH
    software_reset();
}

pub async fn accept_requests<'a>(
    tcp_stack: Stack<'a>,
    rx_buffer: &'a mut [u8],
    tx_buffer: &'a mut [u8],
) -> TcpSocket<'a> {
    let mut tcp_socket = TcpSocket::new(tcp_stack, rx_buffer, tx_buffer);

    info!("Waiting for SSH client...");
    if let Err(e) = tcp_socket
        .accept(IpListenEndpoint {
            addr: None,
            port: 22,
        })
        .await
    {
        error!("connect error: {:?}", e);
        // continue;
        tcp_socket_disable().await;
    }
    info!("Connected, port 22");

    tcp_socket
}

#[embassy_executor::task]
pub async fn wifi_up(
    mut wifi_controller: WifiController<'static>,
    config: &'static SunsetMutex<SSHStampConfig>,
) {
    info!("Device capabilities: {:?}", wifi_controller.capabilities());
    let wifi_ssid = {
        let guard = config.lock().await;
        guard.wifi_ssid.clone()
        // drop guard
    };
    // TODO: No wifi password(s) yet...
    //let wifi_password = config.lock().await.wifi_pw;

    log::debug!("Starting wifi");

    let ssid_string = String::<63>::try_from(wifi_ssid.as_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|_| {
            log::warn!("SSID too long, using default");
            DEFAULT_SSID.into()
        });
    let client_config =
        ModeConfig::AccessPoint(AccessPointConfig::default().with_ssid(ssid_string));

    loop {
        if esp_radio::wifi::ap_state() == WifiApState::Started {
            // wait until we're no longer connected
            wifi_controller.wait_for_event(WifiEvent::ApStop).await;
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(wifi_controller.is_started(), Ok(true)) {
            if let Err(e) = wifi_controller.set_config(&client_config) {
                info!("Failed to set wifi config: {:?}", e);
                Timer::after(Duration::from_millis(1000)).await;
                continue;
            }
            info!("Starting wifi");
            if let Err(e) = wifi_controller.start_async().await {
                info!("Failed to start wifi: {:?}", e);
                Timer::after(Duration::from_millis(1000)).await;
                continue;
            }
            info!("Wifi started!");
        }
        Timer::after(Duration::from_millis(10)).await;
    }
}

pub async fn wifi_controller_disable() -> () {
    // TODO: Correctly disable wifi controller
    // pub async fn wifi_disable(wifi_controller: EspWifiController<'_>) -> (){
    // drop wifi controller
    // esp_wifi::deinit_unchecked()
    // wifi_controller.deinit_unchecked()
    software_reset();
}

use esp_radio::wifi::WifiDevice;
#[embassy_executor::task]
async fn net_up(mut runner: Runner<'static, WifiDevice<'static>>) {
    info!("Bringing up network stack...\n");
    runner.run().await
}

#[embassy_executor::task]
async fn dhcp_server(stack: Stack<'static>, ip: Ipv4Addr) {
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
            log::warn!("Failed to bind DHCP server socket: {e:?}");
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
            log::warn!("DHCP server error: {e:?}");
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}
