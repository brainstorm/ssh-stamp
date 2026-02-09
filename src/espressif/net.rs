// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::net::Ipv4Addr;
use core::str::FromStr;

// use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, Stack, StackResources};
use embassy_net::{IpListenEndpoint, Ipv4Cidr, StaticConfigV4};
use embassy_time::{Duration, Timer};

use esp_hal::peripherals::WIFI;

use esp_hal::rng::Rng;
use esp_println::{dbg, println};

use esp_radio::wifi::WifiEvent;
use esp_radio::wifi::{AccessPointConfig, ModeConfig, WifiApState, WifiController};
use esp_radio::Controller;
// use sunset_async::SunsetMutex;

use core::net::SocketAddrV4;
use edge_dhcp;

use edge_dhcp::{
    io::{self, DEFAULT_SERVER_PORT},
    server::{Server, ServerOptions},
};
use edge_nal::UdpBind;
use edge_nal_embassy::{Udp, UdpBuffers};

// use crate::config::SSHStampConfig;
// use crate::settings::DEFAULT_SSID;
use crate::settings::{DEFAULT_IP, DEFAULT_SSID};

use super::buffered_uart::BufferedUart;

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

use heapless::String;

pub async fn if_up<'a>(
    wifi_controller: Controller<'a>,
    wifi: WIFI<'a>,
    rng: &mut Rng,
    // config:  &'static SunsetMutex<SSHStampConfig>,
    wifi_ssid: String<32>,
) -> Result<Stack<'a>, sunset::Error> {
    let (mut controller, interfaces) =
        esp_radio::wifi::new(&wifi_controller, wifi, Default::default()).unwrap();
    let ap_config =
        ModeConfig::AccessPoint(AccessPointConfig::default().with_ssid(DEFAULT_SSID.into()));
    let res = controller.set_config(&ap_config);
    println!("wifi_set_configuration returned {:?}", res);

    let gw_ip_addr_ipv4 = DEFAULT_IP.clone();

    // let _gw_ip_addr = {
    //     let guard = config.lock().await;
    //     if let Some(ref s) = guard.ip4_static {
    //         embassy_net::Config::ipv4_static(s.clone())
    //     } else {
    //         embassy_net::Config::dhcpv4(Default::default())
    //     }
    // };

    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip_addr_ipv4, 24),
        gateway: Some(gw_ip_addr_ipv4),
        dns_servers: Default::default(),
    });

    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let (ap_stack, _runner) = embassy_net::new(
        interfaces.ap,
        net_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    // spawner.spawn(wifi_up(controller, wifi_ssid)).ok();
    // spawner.spawn(net_up(runner)).ok();
    // spawner.spawn(dhcp_server(ap_stack, gw_ip_addr_ipv4)).ok();
    wifi_up(controller, wifi_ssid).await;
    dhcp_server(ap_stack, gw_ip_addr_ipv4).await;

    loop {
        println!("Checking if link is up...\n");
        if ap_stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    // TODO: Use wifi_manager instead?
    println!(
        "Connect to the AP `ssh-stamp` as a DHCP client with IP: {}",
        gw_ip_addr_ipv4
    );

    Ok(ap_stack)
}

pub async fn accept_requests<'a>(stack: Stack<'a>, _uart: &BufferedUart) -> ! {
    // let rx_buffer = mk_static!([u8; 1536], [0; 1536]);
    // let tx_buffer = mk_static!([u8; 1536], [0; 1536]);
    let mut rx_buffer = [0u8; 1536];
    let mut tx_buffer = [0u8; 1536];
    loop {
        // let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);

        println!("Waiting for SSH client...");

        if let Err(e) = socket
            .accept(IpListenEndpoint {
                addr: None,
                port: 22,
            })
            .await
        {
            println!("connect error: {:?}", e);
            continue;
        }

        println!("Connected, port 22");
        // match crate::serve::handle_ssh_client(&mut socket, uart, pin_channel_ref).await {
        //     Ok(_) => (),
        //     Err(e) => {
        //         println!("SSH client fatal error: {}", e);
        //     }
        // };
    }
}

// #[embassy_executor::task]
async fn wifi_up(
    // mut controller: WifiController<'static>,
    mut controller: WifiController<'_>,
    // config: &'static SunsetMutex<SSHStampConfig>,
    wifi_ssid: String<32>,
) {
    println!("Device capabilities: {:?}", controller.capabilities());
    // let wifi_ssid = {
    //     let guard = config.lock().await;
    //     guard.wifi_ssid.clone()
    //     // drop guard
    // };
    // TODO: No wifi password(s) yet...
    //let wifi_password = config.lock().await.wifi_pw;

    loop {
        if esp_radio::wifi::ap_state() == WifiApState::Started {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::ApStop).await;
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let ssid_string = String::<63>::from_str(&wifi_ssid)
                .unwrap()
                .to_ascii_lowercase();
            let client_config =
                ModeConfig::AccessPoint(AccessPointConfig::default().with_ssid(ssid_string));
            controller.set_config(&client_config).unwrap();
            println!("Starting wifi");
            controller.start_async().await.unwrap();
            println!("Wifi started!");
        }
        Timer::after(Duration::from_millis(10)).await;
    }
}

// #[embassy_executor::task]
// async fn net_up<'a>(mut runner: Runner<'a, WifiDevice<'a>>) {
// println!("Bringing up network stack...\n");
// runner.run().await
// }

// #[embassy_executor::task]
async fn dhcp_server<'a>(stack: Stack<'a>, ip: Ipv4Addr) {
    let mut buf = [0u8; 1500];

    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];

    let buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
    let unbound_socket = Udp::new(stack, &buffers);
    let mut bound_socket = unbound_socket
        .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
        .unwrap();

    loop {
        let res = io::server::run(
            &mut Server::<_, 64>::new_with_et(ip),
            &ServerOptions::new(ip, Some(&mut gw_buf)),
            &mut bound_socket,
            &mut buf,
        )
        .await
        .inspect_err(|e| log::warn!("DHCP server error: {e:?}"));
        Timer::after(Duration::from_millis(500)).await;

        dbg!(res.unwrap());
    }
}
