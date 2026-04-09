// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Network support with app-specific configuration
//!
//! Re-exports from hal-espressif and provides app-specific network initialization.

use crate::config::SSHStampConfig;
use crate::settings::DEFAULT_IP;
use crate::store;

use core::net::Ipv4Addr;
use core::net::SocketAddrV4;

use edge_dhcp::io::{self, DEFAULT_SERVER_PORT};
use edge_dhcp::server::{Server, ServerOptions};
use edge_nal::UdpBind;
use edge_nal_embassy::{Udp, UdpBuffers};
use embassy_executor::Spawner;
use embassy_net::{Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_hal::efuse::Efuse;
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::Controller;
use esp_radio::wifi::{
    AccessPointConfig, AuthMethod, ModeConfig, WifiApState, WifiController, WifiEvent,
};
use hal_espressif::flash;
use heapless::String;
use log::{debug, error, info, warn};
use sunset_async::SunsetMutex;

extern crate alloc;
use alloc::string::String as AllocString;

use hal_espressif::WIFI_PASSWORD_CHARS;

// Re-export functions from hal-espressif
pub use hal_espressif::{
    accept_requests, ap_stack_disable, tcp_socket_disable, wifi_controller_disable,
};

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

/// Brings up the `WiFi` interface.
///
/// # Errors
/// Returns an error if the `WiFi` configuration or initialization fails.
///
/// # Panics
/// Panics if flash storage is not initialized or if persisting the wifi password fails.
pub async fn if_up(
    spawner: Spawner,
    controller: Controller<'static>,
    wifi: WIFI<'static>,
    rng: Rng,
    config: &'static SunsetMutex<SSHStampConfig>,
) -> Result<Stack<'static>, sunset::Error> {
    let wifi_init = &*mk_static!(Controller<'static>, controller);
    let (mut wifi_controller, interfaces) =
        esp_radio::wifi::new(wifi_init, wifi, Config::default())
            .map_err(|_| sunset::error::BadUsage.build())?;

    // Ensure WiFi PSK exists before applying AP config to avoid esp_wifi_set_config errors
    {
        let mut guard = config.lock().await;
        if guard.wifi_pw.is_none() {
            let mut rnd = [0u8; 24];
            for chunk in rnd.chunks_exact_mut(4) {
                chunk.copy_from_slice(&rng.random().to_le_bytes());
            }

            let mut pw = String::<63>::new();
            for &byte in &rnd {
                let _ = pw.push(WIFI_PASSWORD_CHARS[(byte as usize) % 62] as char);
            }

            warn!("wifi_pw missing from config, generated new password");
            guard.wifi_pw = Some(pw);

            let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
                panic!("Flash storage not initialized; cannot persist wifi password");
            };
            let mut flash_storage = flash_storage_guard.lock().await;
            if let Err(e) = store::save(&mut flash_storage, &guard) {
                panic!("Failed to persist generated wifi password: {e:?}");
            }
        }
        info!("WIFI PSK: {}", guard.wifi_pw.as_ref().unwrap());

        // Set MAC address: use configured MAC (may be random sentinel or hardware default)
        let mac = guard
            .resolve_mac()
            .map_err(|_| sunset::error::BadUsage.build())?;
        info!(
            "WIFI MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );
        Efuse::set_mac_address(mac).map_err(|_| sunset::error::BadUsage.build())?;

        print_hostkey_fingerprint(&guard.hostkey);
    }

    let ssid_name = wifi_ssid(config).await;

    let ap_config = ModeConfig::AccessPoint(
        AccessPointConfig::default()
            .with_ssid(AllocString::from(ssid_name.as_str()))
            .with_auth_method(AuthMethod::Wpa2Wpa3Personal)
            .with_password(AllocString::from(wifi_password(config).await.as_str())),
    );
    let res = wifi_controller.set_config(&ap_config);
    debug!("wifi_set_configuration returned {res:?}");

    let gw_ip_addr_ipv4 = DEFAULT_IP;

    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip_addr_ipv4, 24),
        gateway: Some(gw_ip_addr_ipv4),
        dns_servers: heapless::Vec::new(),
    });

    let seed = u64::from(rng.random()) << 32 | u64::from(rng.random());

    let (ap_stack, runner) = embassy_net::new(
        interfaces.ap,
        net_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    let ssid = wifi_ssid(config).await;
    let password = wifi_password(config).await;

    // Convert to static strings for the task
    let ssid_static: &'static str =
        alloc::boxed::Box::leak(alloc::string::String::from(ssid.as_str()).into_boxed_str());
    let password_static: &'static str =
        alloc::boxed::Box::leak(alloc::string::String::from(password.as_str()).into_boxed_str());

    spawner
        .spawn(wifi_up(wifi_controller, ssid_static, password_static))
        .ok();
    spawner.spawn(net_up(runner)).ok();
    spawner.spawn(dhcp_server(ap_stack, gw_ip_addr_ipv4)).ok();

    loop {
        debug!("Checking if link is up");
        if ap_stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    info!("Connect to the AP `{ssid_name}` as a DHCP client with IP: {gw_ip_addr_ipv4}");

    Ok(ap_stack)
}

/// Returns the configured `WiFi` SSID from the config.
///
/// # Panics
/// Panics if `wifi_ssid` is not set in the config or exceeds 63 characters.
pub async fn wifi_ssid(config: &'static SunsetMutex<SSHStampConfig>) -> String<63> {
    let guard = config.lock().await;
    String::<63>::try_from(guard.wifi_ssid.as_str()).expect("SSID should always be set")
}

/// Returns the `WiFi` password from the config.
///
/// # Panics
/// Panics if `wifi_pw` is not set in the config or exceeds 63 characters.
pub async fn wifi_password(config: &'static SunsetMutex<SSHStampConfig>) -> String<63> {
    let guard = config.lock().await;
    let pw_src = guard.wifi_pw.as_ref().expect("wifi_pw should be set");
    String::<63>::try_from(pw_src.as_str()).expect("wifi_pw too long")
}

fn print_hostkey_fingerprint(hostkey: &sunset::SignKey) {
    match hostkey {
        sunset::SignKey::Ed25519(_) => {
            let pubkey = hostkey.pubkey();
            match pubkey.fingerprint(ssh_key::HashAlg::Sha256) {
                Ok(fp) => info!("SSH hostkey fingerprint: {fp}"),
                Err(e) => warn!("Failed to compute fingerprint: {e:?}"),
            }
        }
        sunset::SignKey::AgentEd25519(_) => {
            warn!("Unsupported key type for fingerprint");
        }
    }
}

/// Manages the `WiFi` access point lifecycle.
/// Starts the AP with the configured SSID and password from the config.
/// Handles reconnection if the AP stops.
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

use esp_radio::wifi::Config;
