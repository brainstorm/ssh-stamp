// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use log::{debug, error, info, warn};

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
use esp_radio::Controller;
use esp_radio::wifi::WifiEvent;
use esp_radio::wifi::{
    AccessPointConfig, AuthMethod::Wpa2Wpa3Personal, ModeConfig, WifiApState, WifiController,
};
use heapless::String;
extern crate alloc;
use crate::store;
use alloc::string::String as AllocString;
use storage::flash;
use sunset_async::SunsetMutex;

const PASSWORD_CHARS: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

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

    // Ensure WPA3 PSK exists before applying AP config to avoid esp_wifi_set_config errors
    {
        let mut guard = config.lock().await;
        if guard.wifi_pw.is_none() {
            let mut rnd = [0u8; 24];
            for chunk in rnd.chunks_exact_mut(4) {
                chunk.copy_from_slice(&rng.random().to_le_bytes());
            }

            let mut pw = String::<63>::new();
            for &byte in rnd.iter() {
                let _ = pw.push(PASSWORD_CHARS[(byte as usize) % 62] as char);
            }

            warn!("wifi_pw missing from config, generated new password");
            guard.wifi_pw = Some(pw);

            let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
                panic!("Flash storage not initialized; cannot persist wifi password");
            };
            let mut flash_storage = flash_storage_guard.lock().await;
            if let Err(e) = store::save(&mut flash_storage, &guard).await {
                panic!("Failed to persist generated wifi password: {:?}", e);
            }
        }
        info!("WiFi WPA3 PSK: {}", guard.wifi_pw.as_ref().unwrap());
    }

    let ap_config = ModeConfig::AccessPoint(
        AccessPointConfig::default()
            .with_ssid(AllocString::from(wifi_ssid(config).await.as_str()))
            .with_auth_method(Wpa2Wpa3Personal)
            .with_password(AllocString::from(wifi_password(config).await.as_str())),
    );
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
        debug!("Checking if link is up");
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
    info!("AP Stack disabled: WIP");
    // TODO: Correctly disable/restart AP Stack and/or send messsage to user over SSH
}

pub async fn tcp_socket_disable() -> () {
    // drop tcp stack
    info!("TCP socket disabled: WIP");
    // TODO: Correctly disable/restart tcp socket and/or send messsage to user over SSH
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

pub async fn wifi_ssid(config: &'static SunsetMutex<SSHStampConfig>) -> String<63> {
    // Return the configured SSID if present, otherwise the fixed default.
    let guard = config.lock().await;
    if !guard.wifi_ssid.is_empty() {
        return String::<63>::try_from(guard.wifi_ssid.as_str()).unwrap_or_else(|_| {
            let mut fallback = String::<63>::new();
            fallback.push_str(DEFAULT_SSID).ok();
            fallback
        });
    }

    let mut default = String::<63>::new();
    default.push_str(DEFAULT_SSID).ok();
    default
}

pub async fn wifi_password(config: &'static SunsetMutex<SSHStampConfig>) -> String<63> {
    let guard = config.lock().await;
    match &guard.wifi_pw {
        Some(pw) => String::<63>::try_from(pw.as_str()).unwrap_or_else(|_| {
            panic!("wifi_pw stored value exceeds 63 characters");
        }),
        None => panic!("wifi_pw must be set before calling wifi_password()"),
    }
}

#[embassy_executor::task]
pub async fn wifi_up(
    mut wifi_controller: WifiController<'static>,
    config: &'static SunsetMutex<SSHStampConfig>,
) {
    info!("Device capabilities: {:?}", wifi_controller.capabilities());
    let configured_ssid = {
        let guard = config.lock().await;
        guard.wifi_ssid.clone()
        // drop guard
    };

    debug!("Starting wifi");

    // (PSK generation handled in if_up on first boot)

    let ssid_string = match String::<63>::try_from(configured_ssid.as_str()) {
        Ok(s) => {
            let mut lowered = String::<63>::new();
            for ch in s.as_str().chars() {
                let _ = lowered.push(ch.to_ascii_lowercase());
            }
            lowered
        }
        Err(_) => {
            warn!("SSID too long, using default");
            wifi_ssid(config).await
        }
    };
    let pw_string = wifi_password(config).await;
    let client_config = ModeConfig::AccessPoint(
        AccessPointConfig::default()
            .with_ssid(AllocString::from(ssid_string.as_str()))
            .with_auth_method(Wpa2Wpa3Personal)
            .with_password(AllocString::from(pw_string.as_str())),
    );

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
    info!("Disabling wifi: WIP");
    //software_reset();
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
