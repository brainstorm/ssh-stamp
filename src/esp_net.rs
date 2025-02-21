use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, Stack, StackResources};
use embassy_net::{IpListenEndpoint, Runner};
use embassy_time::{Duration, Timer};

use esp_hal::peripheral::Peripheral;
use esp_hal::peripherals::WIFI;

use esp_hal::rng::Rng;
use esp_hal::uart::Uart;
use esp_hal::Async;
use esp_println::println;

use esp_wifi::wifi::{
    AccessPointConfiguration, Configuration, WifiApDevice, WifiController, WifiDevice,
};
use esp_wifi::wifi::{WifiEvent, WifiState};
use esp_wifi::EspWifiController;

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

pub async fn if_up(
    spawner: Spawner,
    wifi_controller: EspWifiController<'static>,
    wifi: impl Peripheral<P = WIFI> + 'static,
    rng: &mut Rng,
) -> Result<Stack<'static>, sunset::Error> {
    let wifi_init = &*mk_static!(EspWifiController<'static>, wifi_controller);
    let (wifi_ap_interface, _wifi_sta_interface, controller) =
        esp_wifi::wifi::new_ap_sta(wifi_init, wifi).unwrap();

    let config = embassy_net::Config::dhcpv4(Default::default());
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let (ap_stack, runner) = embassy_net::new(
        wifi_ap_interface,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(wifi_up(controller)).ok();
    spawner.spawn(net_up(runner)).ok();

    loop {
        println!("Checking if link is up...\n");
        if ap_stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    // TODO: Offer options for DHCP and static IP, WifiManager-like (minimal) functionality
    println!("Connect to the AP `esp-ssh-rs` and point your ssh client to 192.168.2.1");
    println!("Use a static IP in the range 192.168.2.2 .. 192.168.2.255, use gateway 192.168.2.1");

    Ok(ap_stack)
}

pub async fn accept_requests(
    stack: Stack<'static>,
    uart: Uart<'static, Async>,
) -> Result<(), sunset::Error> {
    let rx_buffer = mk_static!([u8; 1536], [0; 1536]);
    let tx_buffer = mk_static!([u8; 1536], [0; 1536]);

    //loop {
    let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);

    if let Err(e) = socket
        .accept(IpListenEndpoint {
            addr: None,
            port: 22,
        })
        .await
    {
        println!("connect error: {:?}", e);
        //continue;
    }

    println!("Connected, port 22");
    crate::serve::handle_ssh_client(&mut socket, uart).await?;
    //}

    Ok(()) // FIXME: All is fine but not really if we lose connection only once... removed loop to deal with uart copy issues later
           // Probably best handled by some kind of supervisor task and signals instead of a loop anyway?
}

#[embassy_executor::task]
async fn wifi_up(mut controller: WifiController<'static>) {
    println!("Device capabilities: {:?}", controller.capabilities());
    loop {
        if esp_wifi::wifi::wifi_state() == WifiState::ApStarted {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::ApStop).await;
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::AccessPoint(AccessPointConfiguration {
                ssid: "esp-ssh-rs".try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start_async().await.unwrap();
            println!("Wifi started!");
        }
        Timer::after(Duration::from_millis(10)).await;
    }
}

#[embassy_executor::task]
async fn net_up(mut runner: Runner<'static, WifiDevice<'static, WifiApDevice>>) {
    println!("Bringing up network stack...\n");
    runner.run().await
}
