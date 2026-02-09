#![no_std]
#![no_main]

// SPDX-FileCopyrightText: 2025 Julio Beltran Ortega, Anthony Tambasco, Roman Valls Guimera, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::result::Result;
use core::result::Result::Err;
use core::result::Result::Ok;
// use core::error::Error;
use core::future::Future;
use embassy_executor::Spawner;
use embassy_futures::select::{select3, Either3};
use embassy_net::{tcp::TcpSocket, IpListenEndpoint, Stack};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel};
use esp_hal::system::software_reset;
use esp_hal::{
    gpio::Pin,
    interrupt::software::SoftwareInterruptControl,
    peripherals::{GPIO10, GPIO11, UART1},
    peripherals::{SW_INTERRUPT, SYSTIMER, TIMG1, WIFI},
    rng::Rng,
    uart::{Config, RxConfig, Uart},
};
use esp_println::dbg;
use esp_println::println;
use esp_radio::{Controller, InitializationError};
use heapless::String;
use ssh_stamp::{
    config::SSHStampConfig,
    espressif::{
        buffered_uart::BufferedUart,
        // net::accept_requests,
        net::if_up,
        rng,
    },
    serve,
};
use storage::flash;

use static_cell::StaticCell;
use sunset_async::{SSHServer, SunsetMutex};

const UART_BUFFER_SIZE: usize = 4096;
static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();

pub async fn peripherals_wait_for_initialisation<'a>() -> SshStampPeripherals<'a> {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let rng = Rng::new();
    rng::register_custom_rng(rng);
    let timg1 = peripherals.TIMG1;
    let systimer = peripherals.SYSTIMER;

    // Read SSH configuration from Flash (if it exists)
    flash::init(peripherals.FLASH);
    let config = {
        let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
            panic!("Could not acquire flash storage lock");
        };
        let mut flash_storage = flash_storage_guard.lock().await;
        // TODO: Migrate this function/test to embedded-test.
        // Quick roundtrip test for SSHStampConfig
        // ssh_stamp::config::roundtrip_config();
        ssh_stamp::config_storage::load_or_create(&mut flash_storage).await
    }
    .expect("Could not load or create SSHStampConfig");

    static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();
    let config = CONFIG.init(SunsetMutex::new(config));

    let wifi = peripherals.WIFI;
    let gpio10 = peripherals.GPIO10;
    let gpio11 = peripherals.GPIO11;
    let uart1 = peripherals.UART1;
    let sw_interrupt = peripherals.SW_INTERRUPT;
    let ssh_stamp_peripherals = SshStampPeripherals {
        rng: rng,
        wifi: wifi,
        config: config,
        gpio10: gpio10,
        gpio11: gpio11,
        uart1: uart1,
        timg1: timg1,
        systimer: systimer,
        sw_interrupt: sw_interrupt,
    };
    ssh_stamp_peripherals
}

pub async fn peripherals_disable() -> () {
    // drop peripherals
    software_reset();
}

pub async fn wifi_wait_for_initialisation<'a>() -> Result<Controller<'a>, InitializationError> {
    let wifi_controller: Result<Controller<'_>, InitializationError> = esp_radio::init();
    wifi_controller
}

pub async fn wifi_disable() -> () {
    // TODO: Correctly disable wifi controller
    // pub async fn wifi_disable(wifi_controller: EspWifiController<'_>) -> (){
    // drop wifi controller
    // esp_wifi::deinit_unchecked()
    // wifi_controller.deinit_unchecked()
    software_reset();
}

pub struct TcpStackReturn<'a> {
    pub config: SSHStampConfig,
    pub tcp_stack: Stack<'a>,
}
pub async fn tcp_wait_for_initialisation<'a>(s: WifiEnabledConsumed<'a>) -> Stack<'a> {
    let wifi_controller = s.wifi_controller;
    let wifi: WIFI = s.wifi;
    let mut rng = s.rng;
    let wifi_ssid = s.wifi_ssid;
    let tcp_stack = if_up(wifi_controller, wifi, &mut rng, wifi_ssid)
        .await
        .unwrap();
    tcp_stack
}

pub async fn tcp_disable() -> () {
    // drop tcp stack
    software_reset();
}

pub async fn socket_disable() -> () {
    // drop socket
    software_reset();
}

pub async fn ssh_wait_for_initialisation<'server>(
    inbuf: &'server mut [u8; UART_BUFFER_SIZE],
    outbuf: &'server mut [u8; UART_BUFFER_SIZE],
) -> SSHServer<'server> {
    let ssh_server = SSHServer::new(inbuf, outbuf);
    ssh_server
}

pub async fn ssh_disable() -> () {
    // drop wifi controller
    software_reset();
}

pub async fn uart_buffer_wait_for_initialisation() -> &'static BufferedUart {
    UART_BUF.init_with(BufferedUart::new)
}

pub async fn uart_buffer_disable() -> () {
    // disable uart buffer
    software_reset();
}

pub async fn enable_uart<'a, 'b>(
    uart_buf: &'a BufferedUart,
    uart1: UART1<'a>,
    // pin_channel: &'b mut PinChannel<'a>,
    config: &'a SunsetMutex<SSHStampConfig>,
    gpio10: GPIO10<'_>,
    gpio11: GPIO11<'_>,
) where
    'a: 'b,
{
    dbg!("Configuring UART");
    let config_lock = config.lock().await;
    let rx: u8 = config_lock.uart_pins.rx;
    let tx: u8 = config_lock.uart_pins.tx;
    if rx != tx {
        let mut holder10 = Some(gpio10);
        let mut holder11 = Some(gpio11);
        let rx_pin = match rx {
            10 => holder10.take().unwrap().degrade(),
            11 => holder11.take().unwrap().degrade(),
            _ => holder10.take().unwrap().degrade(),
        };
        let tx_pin = match tx {
            10 => holder10.take().unwrap().degrade(),
            11 => holder11.take().unwrap().degrade(),
            _ => holder11.take().unwrap().degrade(),
        };

        // Hardware UART setup
        let uart_config = Config::default().with_rx(
            RxConfig::default()
                .with_fifo_full_threshold(16)
                .with_timeout(1),
        );

        let uart = Uart::new(uart1, uart_config)
            .unwrap()
            .with_rx(rx_pin)
            .with_tx(tx_pin)
            .into_async();
        // Run the main buffered TX/RX loop
        uart_buf.run(uart).await;
    }
    // TODO: Pin config error
}

pub async fn uart_disable() -> () {
    // disable uart
    software_reset();
}

pub async fn idle_wait_for_connection<'a, 'b>(
    ssh_server: &'b SSHServer<'a>,
    chan_pipe: &Channel<NoopRawMutex, serve::SessionType, 1>,
    config: &'a SunsetMutex<SSHStampConfig>,
) -> Result<(), sunset::Error>
where
    'a: 'b,
{
    serve::connection_loop(ssh_server, chan_pipe, config).await
}

pub async fn connection_disable() -> () {
    // disable idle
    software_reset();
}

use crate::serve::SessionType;
use ssh_stamp::serial::serial_bridge;
use sunset_async::ChanInOut;
pub async fn bridge_wait_for_initialisation<'a, 'b>(
    s: UartEnabledConsumed<'a, 'b>,
) -> Result<(), sunset::Error> {
    let bridge = {
        let chan_pipe = s.chan_pipe;
        let session_type = chan_pipe.receive().await;

        match session_type {
            serve::SessionType::Bridge(ch) => {
                let stdio: ChanInOut<'_> = s.ssh_server.stdio(ch).await?;
                let stdio2 = stdio.clone();
                serial_bridge(stdio, stdio2, s.uart_buff).await?
            }
            SessionType::Sftp(_ch) => {
                // Handle SFTP session
                //     todo!()
            }
        };
        Ok(())
    };
    bridge
}

pub async fn bridge_disable() -> () {
    // disable bridge
    software_reset();
}

pub struct SshStampPeripherals<'a> {
    pub rng: Rng,
    pub wifi: WIFI<'a>,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
    pub uart1: UART1<'a>,
    pub timg1: TIMG1<'a>,
    pub systimer: SYSTIMER<'a>,
    pub sw_interrupt: SW_INTERRUPT<'a>,
}

pub struct SshStampInit<'a> {
    pub rng: Rng,
    pub wifi: WIFI<'a>,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
    pub uart1: UART1<'a>,
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32s2")] {
            // TODO: This heap size will crash at runtime, we need to fix this
            // applying ideas from https://github.com/brainstorm/ssh-stamp/pull/41#issuecomment-2964775170
                esp_alloc::heap_allocator!(size: 69 * 1024);
        } else {
                esp_alloc::heap_allocator!(size: 72 * 1024);
        }
    );
    esp_bootloader_esp_idf::esp_app_desc!();
    esp_println::logger::init_logger_from_env();

    let peripherals = peripherals_wait_for_initialisation().await;

    let sw_int = SoftwareInterruptControl::new(peripherals.sw_interrupt);

    cfg_if::cfg_if! {
        if #[cfg(feature = "esp32")] {
            let timg1 = TimerGroup::new(peripherals.timg1);
            esp_rtos::start(timg1.timer0, sw_int);
        } else {
            use esp_hal::timer::systimer::SystemTimer;
            let systimer = SystemTimer::new(peripherals.systimer);
            esp_rtos::start(systimer.alarm0, sw_int.software_interrupt0);
        }
    }

    // TODO: Migrate this function/test to embedded-test.
    // Quick roundtrip test for SSHStampConfig
    // ssh_stamp::config::roundtrip_config();

    let peripherals_enabled_struct = SshStampInit {
        rng: peripherals.rng,
        wifi: peripherals.wifi,
        config: peripherals.config,
        gpio10: peripherals.gpio10,
        gpio11: peripherals.gpio11,
        uart1: peripherals.uart1,
    };

    match peripherals_enabled(peripherals_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("Peripheral error: {}", e);
        }
    }

    peripherals_disable().await;
    // loop{}
    software_reset();
}

pub struct PeripheralsEnabled<'a> {
    pub rng: Rng,
    pub wifi: WIFI<'a>,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub wifi_controller: Controller<'a>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
    pub uart1: UART1<'a>,
}
async fn peripherals_enabled<'a>(s: SshStampInit<'a>) -> Result<(), sunset::Error> {
    let wifi_controller = wifi_wait_for_initialisation().await;

    let peripherals_enabled_struct = PeripheralsEnabled {
        rng: s.rng,
        wifi: s.wifi,
        config: s.config,
        wifi_controller: wifi_controller.unwrap(),
        gpio10: s.gpio10,
        gpio11: s.gpio11,
        uart1: s.uart1,
    };
    match wifi_enabled(peripherals_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("Wifi error: {}", e);
        }
    }

    wifi_disable().await;
    Ok(()) // todo!() return relevant value
}

pub struct WifiEnabledConsumed<'a> {
    pub rng: Rng,
    pub wifi: WIFI<'a>,
    pub wifi_ssid: String<32>,
    pub wifi_controller: Controller<'a>,
}
pub struct WifiEnabled<'a> {
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub tcp_stack: Stack<'a>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
    pub uart1: UART1<'a>,
}
async fn wifi_enabled<'a>(s: PeripheralsEnabled<'a>) -> Result<(), sunset::Error> {
    let wifi_ssid_config = {
        let guard = s.config.lock().await;
        guard.wifi_ssid.clone()
    };
    let wifi_enabled_consumed = WifiEnabledConsumed {
        rng: s.rng,
        wifi: s.wifi,
        wifi_ssid: wifi_ssid_config,
        wifi_controller: s.wifi_controller,
    };
    let tcp_stack = tcp_wait_for_initialisation(wifi_enabled_consumed).await;
    let wifi_enabled_struct = WifiEnabled {
        config: s.config,
        tcp_stack: tcp_stack,
        gpio10: s.gpio10,
        gpio11: s.gpio11,
        uart1: s.uart1,
    };
    match tcp_enabled(wifi_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("TCP stack error: {}", e);
        }
    }
    tcp_disable().await;
    Ok(()) // todo!() return relevant value
}

pub struct TCPEnabled<'a> {
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub tcp_socket: TcpSocket<'a>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
    pub uart1: UART1<'a>,
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
        config: s.config,
        tcp_socket: tcp_socket,
        gpio10: s.gpio10,
        gpio11: s.gpio11,
        uart1: s.uart1,
    };
    match socket_enabled(tcp_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("TCP socket error: {}", e);
        }
    }
    socket_disable().await;
    Ok(()) // todo!() return relevant value
}

pub struct SocketEnabled<'a> {
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: SSHServer<'a>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
    pub uart1: UART1<'a>,
}
async fn socket_enabled<'a>(s: TCPEnabled<'a>) -> Result<(), sunset::Error> {
    // loop {
    // Spawn network tasks to handle incoming connections with demo_common::session()
    let mut inbuf = [0u8; UART_BUFFER_SIZE];
    let mut outbuf = [0u8; UART_BUFFER_SIZE];
    let ssh_server = ssh_wait_for_initialisation(&mut inbuf, &mut outbuf).await;
    let socket_enabled_struct = SocketEnabled {
        config: s.config,
        tcp_socket: s.tcp_socket,
        ssh_server: ssh_server,
        gpio10: s.gpio10,
        gpio11: s.gpio11,
        uart1: s.uart1,
    };
    match ssh_enabled(socket_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("SSH server error: {}", e);
        }
    }

    ssh_disable().await;
    // }
    Ok(()) // todo!() return relevant value
}

pub struct SshEnabled<'a, 'b, CL>
where
    CL: Future<Output = Result<(), sunset::Error>>,
{
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: &'b SSHServer<'a>,
    pub uart1: UART1<'a>,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub chan_pipe: &'b Channel<NoopRawMutex, serve::SessionType, 1>,
    pub connection_loop: CL,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
}
async fn ssh_enabled<'a>(s: SocketEnabled<'a>) -> Result<(), sunset::Error>
// where
    // 'b: 'a,
{
    // loop {
    let chan_pipe = Channel::<NoopRawMutex, serve::SessionType, 1>::new();

    println!("Calling connection_loop from uart_enabled");
    let connection = idle_wait_for_connection(&s.ssh_server, &chan_pipe, &s.config);

    let ssh_enabled_struct = SshEnabled {
        tcp_socket: s.tcp_socket,
        ssh_server: &s.ssh_server,
        config: s.config,
        chan_pipe: &chan_pipe,
        connection_loop: connection,
        uart1: s.uart1,
        gpio10: s.gpio10,
        gpio11: s.gpio11,
    };
    match client_connected(ssh_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("Client connection error: {}", e);
        }
    }

    connection_disable().await;
    // }
    Ok(()) // todo!() return relevant value
}

pub struct ClientConnected<'a, 'b, CL>
where
    CL: Future<Output = Result<(), sunset::Error>>,
{
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: &'b SSHServer<'a>,
    pub chan_pipe: &'b Channel<NoopRawMutex, serve::SessionType, 1>,
    pub connection_loop: CL,
    pub uart_buff: &'a BufferedUart,
    pub uart1: UART1<'a>,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub gpio10: GPIO10<'a>,
    pub gpio11: GPIO11<'a>,
}
async fn client_connected<'a, 'b, CL>(s: SshEnabled<'a, 'b, CL>) -> Result<(), sunset::Error>
// where 'b: 'a {
where
    CL: Future<Output = Result<(), sunset::Error>>,
{
    // loop {
    let uart_buff = uart_buffer_wait_for_initialisation().await;
    let client_connected_struct = ClientConnected {
        tcp_socket: s.tcp_socket,
        ssh_server: s.ssh_server,
        chan_pipe: s.chan_pipe,
        connection_loop: s.connection_loop,
        uart_buff: uart_buff,
        uart1: s.uart1,
        config: s.config,
        gpio10: s.gpio10,
        gpio11: s.gpio11,
    };
    match uart_buffer_ready(client_connected_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("UART buffer error: {}", e);
        }
    }

    uart_buffer_disable().await;
    // }
    Ok(()) // todo!() return relevant value
}

pub struct UartBufferReady<'a, 'b, CL>
where
    CL: Future<Output = Result<(), sunset::Error>>,
{
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: &'b SSHServer<'a>,
    pub uart_buff: &'a BufferedUart,
    pub chan_pipe: &'b Channel<NoopRawMutex, serve::SessionType, 1>,
    pub connection_loop: CL,
}
async fn uart_buffer_ready<'a, 'b, CL>(s: ClientConnected<'a, 'b, CL>) -> Result<(), sunset::Error>
where
    // 'b: 'a,
    CL: Future<Output = Result<(), sunset::Error>>,
{
    // loop {
    let _uart = enable_uart(s.uart_buff, s.uart1, &s.config, s.gpio10, s.gpio11);

    let uart_buffer_ready_struct = UartBufferReady {
        tcp_socket: s.tcp_socket,
        ssh_server: s.ssh_server,
        chan_pipe: s.chan_pipe,
        // uart_pins: &s.uart_pins,
        uart_buff: s.uart_buff,
        connection_loop: s.connection_loop,
    };
    match uart_enabled(uart_buffer_ready_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("UART error: {}", e);
        }
    }

    uart_disable().await;
    // }
    Ok(()) // todo!() return relevant value
}

pub struct UartEnabledConsumed<'a, 'b> {
    pub uart_buff: &'a BufferedUart,
    pub ssh_server: &'b SSHServer<'a>,
    pub chan_pipe: &'b Channel<NoopRawMutex, serve::SessionType, 1>,
}
pub struct UartEnabled<'a, 'b, CL, BR>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    BR: Future<Output = Result<(), sunset::Error>>,
{
    pub ssh_server: &'b SSHServer<'a>,
    pub bridge: BR,
    pub connection_loop: CL,
    pub tcp_socket: TcpSocket<'a>,
}

async fn uart_enabled<'a, 'b, CL>(s: UartBufferReady<'a, 'b, CL>) -> Result<(), sunset::Error>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    'a: 'b,
{
    // loop {
    let uart_enabled_consumed = UartEnabledConsumed {
        uart_buff: s.uart_buff,
        ssh_server: s.ssh_server,
        chan_pipe: s.chan_pipe,
    };

    println!("Setting up serial bridge");
    let bridge = bridge_wait_for_initialisation(uart_enabled_consumed);

    let uart_enabled_struct = UartEnabled {
        ssh_server: s.ssh_server,
        bridge: bridge,
        connection_loop: s.connection_loop,
        tcp_socket: s.tcp_socket,
    };
    match bridge_connected(uart_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("Bridge error: {}", e);
        }
    }

    bridge_disable().await;
    // }

    Ok(()) // todo!() return relevant value
}

async fn bridge_connected<'a, 'b, CL, BR>(
    s: UartEnabled<'a, 'b, CL, BR>,
) -> Result<(), sunset::Error>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    BR: Future<Output = Result<(), sunset::Error>>,
    'a: 'b,
{
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
