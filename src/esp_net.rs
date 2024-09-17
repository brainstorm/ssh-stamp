// https://github.com/esp-rs/esp-hal/blob/main/examples/src/bin/wifi_embassy_access_point.rs
// https://github.com/embassy-rs/embassy/blob/main/examples/nrf52840/src/bin/wifi_esp_hosted.rs
use core::fmt::Error;

use embassy_executor::Spawner;
use embassy_net::{
    tcp::TcpSocket,
    Config,
    IpListenEndpoint,
    Ipv4Address,
    Ipv4Cidr,
    Stack,
    StaticConfigV4,
    StackResources
};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::timer::systimer::{SystemTimer, Target};
use esp_hal::{
    clock::ClockControl,
    peripherals::Peripherals,
    rng::Rng,
    system::SystemControl,
    timer::timg::TimerGroup,
};
use esp_println::println;
use esp_wifi::wifi::{WifiEvent, WifiState};
use esp_wifi::{
    initialize,
    wifi::{
        AccessPointConfiguration,
        Configuration,
        WifiApDevice,
        WifiController,
        WifiDevice,
    },
    EspWifiInitFor,
};

use crate::settings::MTU;
//use static_cell::make_static;

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

pub async fn if_up(spawner: Spawner) -> Result<Stack<WifiDevice<'static, WifiApDevice>>, Error>
{
    esp_println::logger::init_logger_from_env();

    let peripherals = Peripherals::take();

    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();

    let timg0 = TimerGroup::new(peripherals.TIMG0, &clocks);

    let init = initialize(
        EspWifiInitFor::Wifi,
        timg0.timer0,
        Rng::new(peripherals.RNG),
        peripherals.RADIO_CLK,
        &clocks,
    )
    .unwrap();

    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiApDevice).unwrap();

    let systimer = SystemTimer::new(peripherals.SYSTIMER).split::<Target>(); // TODO: Substitute by Alarm::new instead of .split()...
    esp_hal_embassy::init(&clocks, systimer.alarm0);

    let config = Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 2, 1), 24),
        gateway: Some(Ipv4Address::from_bytes(&[192, 168, 2, 1])),
        dns_servers: Default::default(),
    });

    let seed = 1234; // very random, very secure seed

    // TODO: Revisit/review this carefully, using make_static! instead of mk_static!... ?
    // Init network stack
    let stack = &*mk_static!(
        Stack<WifiDevice<'static, WifiApDevice>>,
        Stack::new(
            wifi_interface,
            config,
            mk_static!(StackResources<3>, StackResources::<3>::new()),
            seed
        )
    );
    
    // let stack = make_static!(
    //     Stack<WifiApDevice && WifiApDevice<'_', WifiDevice>>,
    //     Stack::new(
    //         wifi_interface,
    //         config,
    //         make_static!(StackResources<3>, StackResources::<3>::new()),
    //         seed
    //     )
    // );

    spawner.spawn(wifi_up(controller)).ok();
    spawner.spawn(net_up(stack)).ok();

    loop {
        println!("Checking if link is up...\n");
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    // TODO: Offer options for DHCP and static IP, WifiManager-like (minimal) functionality
    println!("Connect to the AP `esp-ssh-rs` and point your ssh client to 192.168.2.1");
    println!("Use a static IP in the range 192.168.2.2 .. 192.168.2.255, use gateway 192.168.2.1");

    Ok(stack)
}

pub async fn accept_requests(mut socket: TcpSocket<'static>) {
    // accept() connections..
    //let mut debug_socket = io::DebuggableTcpSocket::<'static>(socket);
    loop {
        let mut r = socket.
        accept(IpListenEndpoint {
            addr: None,
            port: 22,
        }).await;

        println!("Connected, port 22");

        if let Err(e) = r {
            println!("connect error: {:?}", e);
            continue;
        }

        let mut buffer = [0u8; 1024];
        loop {
            match socket.read(&mut buffer).await {
                Ok(0) => {
                    println!("read EOF");
                    break;
                }
                Ok(len) => {
                    println!("Command received with length: {:?}", len);
                    crate::serve::handle_ssh_client(socket);
                }
                Err(e) => {
                    println!("read error: {:?}", e);
                    break;
                }
            };
        }

        let r = socket.flush().await;
        if let Err(e) = r {
            println!("flush error: {:?}", e);
        }

        // TODO: Check socket close() and abort() requirements for SSH server
        Timer::after(Duration::from_millis(1000)).await;

        socket.close();
        Timer::after(Duration::from_millis(1000)).await;

        socket.abort();
    }
}

#[embassy_executor::task]
async fn wifi_up(mut controller: WifiController<'static>) {
    println!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        match esp_wifi::wifi::get_wifi_state() {
            WifiState::ApStarted => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::ApStop).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::AccessPoint(AccessPointConfiguration {
                ssid: "esp-ssh-rs".try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start().await.unwrap();
            println!("Wifi started!");
        }
    }
}

#[embassy_executor::task]
async fn net_up(stack: &'static Stack<WifiDevice<'static, WifiApDevice>>) {
    println!("Bringing up network stack...\n");
    stack.run().await
}