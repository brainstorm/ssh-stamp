#![no_std]
#![no_main]
use core::result::Result;
use core::result::Result::Ok;
use core::result::Result::Err;
// use core::error::Error;
use core::future::Future;
use embassy_futures::select::{select3, Either3};
use embassy_executor::Spawner;
use embassy_net::{ Stack, tcp::TcpSocket, IpListenEndpoint};
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    channel::Channel
};
use esp_println::println;
use esp_hal::{
    gpio::Pin,
    // interrupt::{software::SoftwareInterruptControl, Priority},
    // peripherals::Peripherals,
    // peripherals::UART1,
    peripherals::{RADIO_CLK, WIFI, TIMG0, TIMG1, SYSTIMER},
    peripherals::{GPIO10,GPIO11},
    rng::Rng,
    timer::timg::TimerGroup,
    // uart::{Config, RxConfig, Uart},
};
use heapless::String;
use static_cell::StaticCell;
use esp_wifi::{InitializationError, EspWifiController};
use esp_storage::FlashStorage;
use sunset_async::{SunsetMutex, SSHServer};
use ssh_stamp::{
    pins::{GPIOConfig, PinChannel},
    config::SSHStampConfig,
    espressif::{
        buffered_uart::BufferedUart,
        // net::accept_requests,
        net::if_up,
        rng,
    },
    storage::Fl,
    serve,
};

const UART_BUFFER_SIZE: usize = 4096;
static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();



pub async fn peripherals_wait_for_initialisation<'a>() -> SshStampPeripherals<'a>{
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let rng = Rng::new(peripherals.RNG);
    rng::register_custom_rng(rng);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let timg1 = peripherals.TIMG1;
    let systimer = peripherals.SYSTIMER;
    let radio_clock = peripherals.RADIO_CLK;

    // Read SSH configuration from Flash (if it exists)
        let mut flash_storage = Fl::new(FlashStorage::new());
        let config = ssh_stamp::storage::load_or_create(&mut flash_storage).await;

        static FLASH: StaticCell<SunsetMutex<Fl>> = StaticCell::new();
        let _flash = FLASH.init(SunsetMutex::new(flash_storage));

        static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();
        let config = CONFIG.init(SunsetMutex::new(config.unwrap()));

    let wifi = peripherals.WIFI;
    let gpio10 = peripherals.GPIO10;
    let gpio11 = peripherals.GPIO11;
    let ssh_stamp_peripherals = SshStampPeripherals {
        rng: rng,
        timg0: timg0,
        radio_clock: radio_clock,
        wifi: wifi,
        config: config,
        gpio10: gpio10,
        gpio11: gpio11,
        timg1: timg1,
        systimer: systimer,
    };
    ssh_stamp_peripherals
}

pub async fn peripherals_disable() -> (){
    // drop peripherals
}

pub async fn wifi_wait_for_initialisation(s:  PeripheralsEnabledConsumed<'_>) -> Result<EspWifiController<'_>, InitializationError>{
    let rng = s.rng;
    let timg0 = s.timg0;
    let radio_clock = s.radio_clock;
    let wifi_controller: Result<EspWifiController<'_>, InitializationError> = esp_wifi::init(timg0.timer0, rng, radio_clock);
    wifi_controller
}

pub async fn wifi_disable() -> (){
    // drop wifi controller
}

pub struct TcpStackReturn<'a> {
    pub config: SSHStampConfig,
    pub tcp_stack: Stack<'a>,
}
pub async fn tcp_wait_for_initialisation<'a>(s: WifiEnabledConsumed<'a>) ->  Stack<'a> {
    let spawner: Spawner = s.spawner;
    let wifi_controller = s.wifi_controller;
    let wifi: WIFI = s.wifi;
    let mut rng = s.rng;
    let wifi_ssid = s.wifi_ssid;
    let tcp_stack = if_up(spawner, wifi_controller, wifi, &mut rng, wifi_ssid)
            .await
            .unwrap();
    tcp_stack
}

pub async fn tcp_disable() -> (){
    // drop tcp stack
}

pub async fn socket_disable() -> (){
    // drop socket
}


pub async fn ssh_wait_for_initialisation<'server>(inbuf: &'server mut [u8; UART_BUFFER_SIZE], outbuf: &'server mut [u8; UART_BUFFER_SIZE]) -> SSHServer<'server>{
    let ssh_server = SSHServer::new(inbuf, outbuf);
    ssh_server
}

pub async fn ssh_disable() -> (){

    // drop wifi controller
}

pub async fn uart_pins_wait_for_config<'a>(s: SshEnabledConsumed<'a>) ->  PinChannel<'a> {
    let serde_pin_config = {
        let guard = s.config.lock().await;
        guard.uart_pins.clone()
    };
    let pin10 = s.gpio10.degrade();
    let pin11 = s.gpio11.degrade();

    let available_gpios = GPIOConfig {
        gpio10: Some(pin10),
        gpio11: Some(pin11),
    };
    let pin_channel_ref = PinChannel::new(serde_pin_config, available_gpios);
    pin_channel_ref
}

pub async fn uart_pins_disable() -> (){
    // disable uart pins
}

pub async fn uart_buffer_wait_for_initialisation() -> &'static BufferedUart {
    UART_BUF.init_with(BufferedUart::new)
}

pub async fn uart_buffer_disable() -> () {
    // disable uart buffer
}

pub async fn idle_wait_for_connection<'a, 'b>(s: UartEnabledConsumed<'a>,  ssh_server: &'b SSHServer<'a>, pin_channel: PinChannel<'a>) -> Result<(), sunset::Error> where 'a:'b{
    let chan_pipe = s.chan_pipe;
    serve::connection_loop(ssh_server, &chan_pipe, pin_channel).await
}

pub async fn idle_disable() -> () {
    // disable idle
}

use ssh_stamp::serial::serial_bridge;
use sunset_async::ChanInOut;
pub async fn bridge_wait_for_initialisation<'a, 'b>(s: ClientConnectedConsumed<'a, 'b>) -> Result<(), sunset::Error>{
    let chan_pipe = Channel::<NoopRawMutex, serve::SessionType, 1>::new();
    let bridge = {
        let session_type = chan_pipe.receive().await;

        match session_type {
            serve::SessionType::Bridge(ch) => {
                let stdio: ChanInOut<'_> = s.ssh_server.stdio(ch).await?;
                let stdio2 = stdio.clone();
                serial_bridge(stdio, stdio2, s.uart_buff).await?
            }
            // SessionType::Sftp(_ch) => {
            //     // Handle SFTP session
            //     todo!()
            // }
        };
        Ok(())
    };
    bridge
}

pub async fn bridge_disable() -> () {
    // disable bridge
}


pub struct SshStampPeripherals<'a> {
    pub rng: Rng,
    pub timg0: TimerGroup<'a, TIMG0<'a>>,
    pub radio_clock: RADIO_CLK<'a>,
    pub wifi: WIFI<'a>,
    pub config:  &'a SunsetMutex<SSHStampConfig>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
    pub timg1: TIMG1<'a>,
    pub systimer: SYSTIMER<'a>,
}

pub struct SshStampInit<'a> {
    pub rng: Rng,
    pub timg0: TimerGroup<'a, TIMG0<'a>>,
    pub radio_clock: RADIO_CLK<'a>,
    pub wifi: WIFI<'a>,
    pub config:  &'a SunsetMutex<SSHStampConfig>,
    pub spawner: Spawner,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
}

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32s2")] {
            // TODO: This heap size will crash at runtime (only for the ESP32S2), we need to fix this
            // applying ideas from https://github.com/brainstorm/ssh-stamp/pull/41#issuecomment-2964775170
                esp_alloc::heap_allocator!(size: 69 * 1024);
        } else {
                esp_alloc::heap_allocator!(size: 72 * 1024);
        }
    );
    esp_bootloader_esp_idf::esp_app_desc!();
    esp_println::logger::init_logger_from_env();

    let peripherals = peripherals_wait_for_initialisation().await;

    cfg_if::cfg_if! {
        if #[cfg(feature = "esp32")] {
            let timg1 = TimerGroup::new(peripherals.timg1);
            esp_hal_embassy::init(timg1.timer0);
        } else {
            use esp_hal::timer::systimer::SystemTimer;
            let systimer = SystemTimer::new(peripherals.systimer);
            esp_hal_embassy::init(systimer.alarm0);
        }
    }

    // TODO: Migrate this function/test to embedded-test.
    // Quick roundtrip test for SSHStampConfig
    // ssh_stamp::config::roundtrip_config();

    let peripherals_enabled_struct = SshStampInit {
        rng: peripherals.rng,
        timg0: peripherals.timg0,
        radio_clock: peripherals.radio_clock,
        wifi: peripherals.wifi,
        config: peripherals.config,
        spawner: spawner,
        gpio10: peripherals.gpio10,
        gpio11: peripherals.gpio11,
        };

    match peripherals_enabled(peripherals_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("Peripheral error: {}", e);
        }
    }

    peripherals_disable().await;
    loop{}
    //reset?

}


pub struct PeripheralsEnabledConsumed<'a> {
    pub rng: Rng,
    pub timg0: TimerGroup<'a, TIMG0<'a>>,
    pub radio_clock: RADIO_CLK<'a>,
}
pub struct PeripheralsEnabled<'a> {
    pub rng: Rng,
    pub wifi: WIFI<'a>,
    pub config:  &'a SunsetMutex<SSHStampConfig>,
    pub spawner: Spawner,
    pub wifi_controller: EspWifiController<'a>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
}
async fn peripherals_enabled<'a>(s: SshStampInit<'a>) -> Result<(), sunset::Error> {
    let peripherals_enabled_consumed = PeripheralsEnabledConsumed {
        rng: s.rng,
        timg0: s.timg0,
        radio_clock: s.radio_clock,
        };
    let wifi_controller = wifi_wait_for_initialisation(peripherals_enabled_consumed).await;

    let peripherals_enabled_struct = PeripheralsEnabled {
        rng: s.rng,
        wifi: s.wifi,
        config: s.config,
        spawner: s.spawner,
        wifi_controller: wifi_controller.unwrap(),
        gpio10: s.gpio10,
        gpio11: s.gpio11,
        };
    match wifi_enabled(peripherals_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("TCP error: {}", e);
        }
    }

    Ok(())
    wifi_disable().await;
}

pub struct WifiEnabledConsumed<'a> {
    pub rng: Rng,
    pub wifi: WIFI<'a>,
    // pub config:  &'a SunsetMutex<SSHStampConfig>,
    pub wifi_ssid: String<32>,
    pub spawner: Spawner,
    pub wifi_controller: EspWifiController<'a>,
}
pub struct WifiEnabled<'a> {
    pub rng: Rng,
    pub config:  &'a SunsetMutex<SSHStampConfig>,
    pub tcp_stack: Stack<'a>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
}
async fn wifi_enabled<'a>(s: PeripheralsEnabled<'a>) -> Result<(), sunset::Error> {
    let wifi_ssid_config = {
        let guard = s.config.lock().await;
        guard.wifi_ssid.clone()
    };
    let wifi_enabled_consumed = WifiEnabledConsumed {
        rng: s.rng,
        wifi: s.wifi,
        // config: s.config,
        wifi_ssid: wifi_ssid_config,
        spawner: s.spawner,
        wifi_controller: s.wifi_controller,
        };
    let tcp_stack = tcp_wait_for_initialisation(wifi_enabled_consumed).await;
    let wifi_enabled_struct = WifiEnabled {
        rng: s.rng,
        config: s.config,
        tcp_stack: tcp_stack,
        gpio10: s.gpio10,
        gpio11: s.gpio11,
        };
    match tcp_enabled(wifi_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("SSH error: {}", e);
        }
    Ok(())
    }
    tcp_disable().await;
}

pub struct TCPEnabled<'a> {
    pub rng: Rng,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub tcp_socket: TcpSocket<'a>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
}
async fn tcp_enabled<'a>(s: WifiEnabled<'a>) -> Result<(), sunset::Error> {

    let mut rx_buffer = [0u8; 1536];
    let mut tx_buffer = [0u8; 1536];
    let mut tcp_socket = TcpSocket::new(s.tcp_stack, &mut rx_buffer, &mut tx_buffer);

    println!("Waiting for SSH client...");
    if let Err(e) = tcp_socket
        .accept(IpListenEndpoint {
            addr: None,
            port: 22,
        })
        .await
    {
        println!("connect error: {:?}", e);
        socket_disable().await;
    }
    println!("Connected, port 22");
    let tcp_enabled_struct = TCPEnabled {
        rng: s.rng,
        config: s.config,
        tcp_socket: tcp_socket,
        gpio10: s.gpio10,
        gpio11: s.gpio11,
        };
    match socket_enabled(tcp_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("Wifi error: {}", e);
        }
    }
    socket_disable().await;
    Ok(()) // todo!() return relevant value
 }

pub struct SocketEnabled<'a> {
    pub rng: Rng,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: SSHServer<'a>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
}
async fn socket_enabled<'a>(s: TCPEnabled<'a>) -> Result<(), sunset::Error> {
    // loop {
        // Spawn network tasks to handle incoming connections with demo_common::session()
        let mut inbuf = [0u8; UART_BUFFER_SIZE];
        let mut outbuf = [0u8; UART_BUFFER_SIZE];
        let ssh_server = ssh_wait_for_initialisation(&mut inbuf, &mut outbuf).await;
        let wifi_enabled_struct = SocketEnabled {
            rng: s.rng,
            config: s.config,
            tcp_socket: s.tcp_socket,
            ssh_server: ssh_server,
            gpio10: s.gpio10,
            gpio11: s.gpio11,
            };
        match ssh_enabled(wifi_enabled_struct).await {
            Ok(_) => (),
            Err(e) => {
                println!("Wifi error: {}", e);
            }
        }

        ssh_disable().await;
    // }
    Ok(())

}


pub struct SshEnabledConsumed<'a> {
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
    pub config: &'a SunsetMutex<SSHStampConfig>,
}

pub struct SshEnabled<'a> {
    pub rng: Rng,
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: SSHServer<'a>,
    pub uart_pins: PinChannel<'a>,
}

async fn ssh_enabled<'a>(s: SocketEnabled<'a>) -> Result<(), sunset::Error> {
    // loop {
        let ssh_enabled_consumed = SshEnabledConsumed {
            config: s.config,
            gpio10: s.gpio10,
            gpio11: s.gpio11,
        };

        let uart_pins = uart_pins_wait_for_config(ssh_enabled_consumed).await;

        let ssh_enabled_struct = SshEnabled {
            rng: s.rng,
            tcp_socket: s.tcp_socket,
            ssh_server: s.ssh_server,
            uart_pins: uart_pins,
        };
        match uart_configured(ssh_enabled_struct).await {
            Ok(_) => (),
            Err(e) => {
                println!("UART pin error: {}", e);
            }
        }

        uart_pins_disable().await;
    // }
    Ok(())

}

pub struct UartConfigured<'a> {
    pub rng: Rng,
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: SSHServer<'a>,
    pub uart_pins: PinChannel<'a>,
    pub uart_buff: &'a BufferedUart,
}
async fn uart_configured<'a>(s: SshEnabled<'a>) -> Result<(), sunset::Error> {// where 'b: 'a {
    // loop {
        let uart_buff = uart_buffer_wait_for_initialisation().await;
        let uart_configured_struct = UartConfigured {
            rng: s.rng,
            tcp_socket: s.tcp_socket,
            ssh_server: s.ssh_server,
            uart_pins: s.uart_pins,
            uart_buff: uart_buff,
        };
        match uart_enabled(uart_configured_struct).await {
            Ok(_) => (),
            Err(e) => {
                println!("Uart buffer error: {}", e);
            }
        }

        uart_buffer_disable().await;
    // }
    Ok(())
}



pub struct UartEnabledConsumed<'a>{
    pub rng: Rng,
    pub uart_buff: &'a BufferedUart,
    pub chan_pipe: Channel<NoopRawMutex, serve::SessionType, 1>,
}
pub struct UartEnabled<'a, 'b, CL> where CL: Future<Output = Result<(), sunset::Error>>{
    pub rng: Rng,
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: &'b SSHServer<'a>,
    pub uart_buff: &'a BufferedUart,
    pub connection_loop:  CL,
}
async fn uart_enabled<'a, 'b>(s: UartConfigured<'a>) -> Result<(), sunset::Error> where 'b: 'a{
    // loop {
        let chan_pipe = Channel::<NoopRawMutex, serve::SessionType, 1>::new();

        let uart_enabled_consumed = UartEnabledConsumed {
            rng: s.rng,
            uart_buff: s.uart_buff,
            chan_pipe: chan_pipe,
        };

        println!("Calling connection_loop from uart_enabled");
        let connection = idle_wait_for_connection(uart_enabled_consumed, &s.ssh_server, s.uart_pins);

        let uart_enabled_struct = UartEnabled {
            rng: s.rng,
            tcp_socket: s.tcp_socket,
            ssh_server: &s.ssh_server,
            uart_buff: s.uart_buff,
            connection_loop: connection,
        };
        match client_connected(uart_enabled_struct).await {
            Ok(_) => (),
            Err(e) => {
                println!("Client connection error: {}", e);
            }
        }

        idle_disable().await;
    // }
    Ok(()) // todo!() return relevant value
}

pub struct ClientConnectedConsumed<'a, 'b> {
    pub rng: Rng,
    pub uart_buff: &'a BufferedUart,
    pub ssh_server: &'b SSHServer<'a>,
}
pub struct ClientConnected<'a, 'b, CL, BR> where CL: Future<Output = Result<(), sunset::Error>>, BR: Future<Output = Result<(), sunset::Error>>{
    pub ssh_server: &'b SSHServer<'a>,
    pub bridge:  BR,
    pub connection_loop: CL,
    pub tcp_socket: TcpSocket<'a>,

}

async fn client_connected<'a, 'b, CL>(s: UartEnabled<'a, 'b, CL> )  -> Result<(), sunset::Error> where CL: Future<Output = Result<(), sunset::Error>>, 'a:'b{
    // loop {
        let mut rx_buffer = [0u8; 1536];
        let mut tx_buffer = [0u8; 1536];
        let socket = TcpSocket::new(s.tcp_stack, &mut rx_buffer, &mut tx_buffer);
        let client_connected_consumed = ClientConnectedConsumed {
            rng: s.rng,
            uart_buff: s.uart_buff,
            ssh_server: s.ssh_server,
        };

        let bridge = bridge_wait_for_initialisation(client_connected_consumed);
        let client_connected_struct = ClientConnected {
            ssh_server: s.ssh_server,
            bridge: bridge,
            connection_loop: s.connection_loop,
            tcp_socket: socket,
        };
        match bridge_connected(client_connected_struct).await {
            Ok(_) => (),
            Err(e) => {
                println!("Bridge error: {}", e);
            }
        }

        bridge_disable().await;
    // }

    Ok(())
}

async fn bridge_connected<'a, 'b, CL, BR>(s:ClientConnected<'a, 'b, CL, BR>) -> Result<(), sunset::Error> where CL: Future<Output = Result<(), sunset::Error>>, BR: Future<Output = Result<(), sunset::Error>>, 'a:'b{
    let mut tcp_socket = s.tcp_socket;
    let (mut rsock, mut wsock) = tcp_socket.split();
    println!("Running server from handle_ssh_client()");
    let server = s.ssh_server.run(&mut rsock, &mut wsock);
    let connection_loop = s.connection_loop;
    let bridge = s.bridge;
    println!("Main select() in bridge_connected()");
    match select3(server, connection_loop, bridge).await {
        Either3::First(r) => r,
        Either3::Second(r) => r,
        Either3::Third(r) => r,
    }?;
    Result::Ok(())
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
