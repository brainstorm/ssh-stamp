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
use embassy_futures::select::{Either3, select3};
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel};
use esp_hal::interrupt::{Priority, software::SoftwareInterruptControl};
use esp_hal::system::software_reset;
use esp_hal::{peripherals::WIFI, rng::Rng};
use esp_println::println;
use esp_radio::Controller;
use esp_rtos::embassy::InterruptExecutor;
use ssh_stamp::{
    config::SSHStampConfig,
    espressif::{
        buffered_uart::{BufferedUart, UART_BUF, UartPins, uart_task},
        net, rng,
    },
    serve,
    settings::UART_BUFFER_SIZE,
};
use static_cell::StaticCell;
use storage::flash;
use sunset_async::{SSHServer, SunsetMutex};

cfg_if::cfg_if! {
   if #[cfg(feature = "esp32")] {
        use esp_hal::timer::timg::TimerGroup;
   }
}

pub async fn peripherals_disable() -> () {
    // drop peripherals
    software_reset();
}

pub struct SshStampInit<'a> {
    pub rng: Rng,
    pub wifi: WIFI<'a>,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub uart_buf: &'a BufferedUart,
    pub spawner: Spawner,
}
static INT_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new(); // 0 is used for esp_rtos
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    println!("HSM: main");
    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32s2")] {
            // TODO: This heap size will crash at runtime (only for the ESP32S2), we need to fix this
            // applying ideas from https://github.com/brainstorm/ssh-stamp/pull/41#issuecomment-2964775170
            esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 72 * 1024);
        } else {
                esp_alloc::heap_allocator!(size: 72 * 1024);
        }
    );
    esp_bootloader_esp_idf::esp_app_desc!();
    esp_println::logger::init_logger_from_env();
    println!("HSM: Initialising peripherals ");

    // System init
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let rng = Rng::new();
    rng::register_custom_rng(rng);

    println!("Initialising flash ");
    // Read SSH configuration from Flash (if it exists)
    flash::init(peripherals.FLASH);

    println!("Loading config ");
    let flash_config = {
        let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
            panic!("Could not acquire flash storage lock");
        };
        let mut flash_storage = flash_storage_guard.lock().await;
        // TODO: Migrate this function/test to embedded-test.
        // Quick roundtrip test for SSHStampConfig
        // ssh_stamp::config::roundtrip_config();
        ssh_stamp::store::load_or_create(&mut flash_storage).await
    }
    .expect("Could not load or create SSHStampConfig");

    println!("Initialising config ");
    static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();
    let config: &SunsetMutex<SSHStampConfig> = CONFIG.init(SunsetMutex::new(flash_config));

    println!("Initialising gpio ");
    // Only certain GPIO are available for each target.
    // Pins are selected at compile time based on the target chip.
    cfg_if::cfg_if!(
        if #[cfg(any(feature = "esp32"))]{
            let pins = UartPins {
                rx: peripherals.GPIO13.into(),
                tx: peripherals.GPIO14.into(),
            };
        } else if #[cfg(feature = "esp32c2")] {
            let pins = UartPins {
                rx: peripherals.GPIO9.into(),
                tx: peripherals.GPIO10.into(),
            };
        } else if #[cfg(feature = "esp32c3")] {
            let pins = UartPins {
                rx: peripherals.GPIO20.into(),
                tx: peripherals.GPIO21.into(),
            };
        } else {
            let pins = UartPins {
                rx: peripherals.GPIO10.into(),
                tx: peripherals.GPIO11.into(),
            };
        }
    );

    println!("Initialising timers ");
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    cfg_if::cfg_if! {
       if #[cfg(feature = "esp32")] {
            // TODO: Test this feature configuration
            let timg1 = TimerGroup::new(peripherals.TIMG1);
            esp_rtos::start(timg1.timer0);
       } else if #[cfg(any(feature = "esp32s2", feature = "esp32s3"))] {
            // TODO: Test this feature configuration
           use esp_hal::timer::systimer::SystemTimer;
           let systimer = SystemTimer::new(peripherals.SYSTIMER);
           esp_rtos::start(systimer.alarm0);
       } else {
           use esp_hal::timer::systimer::SystemTimer;
           let systimer = SystemTimer::new(peripherals.SYSTIMER);
           esp_rtos::start(systimer.alarm0, sw_int.software_interrupt0);
       }
    }

    // Set up software buffered UART to run in a higher priority InterruptExecutor
    // Must be higher priority than esp_rtos (Priority1)
    let uart_buf = UART_BUF.init_with(BufferedUart::new);
    let interrupt_executor =
        INT_EXECUTOR.init_with(|| InterruptExecutor::new(sw_int.software_interrupt1));
    cfg_if::cfg_if! {
        if #[cfg(any(feature = "esp32", feature = "esp32s2", feature = "esp32s3"))] {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority3);
        } else {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority10);
        }
    }

    // Use the same config reference for UART task.
    // Pass GPIO peripherals which can then be selected from config values
    interrupt_spawner
        .spawn(uart_task(uart_buf, peripherals.UART1, config, pins))
        .unwrap();

    let peripherals_enabled_struct = SshStampInit {
        rng,
        wifi: peripherals.WIFI,
        config,
        spawner,
        uart_buf,
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
    pub controller: Controller<'static>,
    pub uart_buf: &'a BufferedUart,
    pub spawner: Spawner,
}

async fn peripherals_enabled(s: SshStampInit<'static>) -> Result<(), sunset::Error> {
    println!("HSM: peripherals_enabled");
    let controller = esp_radio::init().unwrap();

    let peripherals_enabled_struct = PeripheralsEnabled {
        rng: s.rng,
        wifi: s.wifi,
        config: s.config,
        controller,
        uart_buf: s.uart_buf,
        spawner: s.spawner,
    };
    match wifi_controller_enabled(peripherals_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("Wifi controller error: {}", e);
        }
    }

    net::wifi_controller_disable().await;
    Ok(()) // todo!() return relevant value
}

pub struct WifiControllerEnabled<'a> {
    pub rng: Rng,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub uart_buf: &'a BufferedUart,
    pub tcp_stack: Stack<'a>,
}

pub async fn wifi_controller_enabled(s: PeripheralsEnabled<'static>) -> Result<(), sunset::Error> {
    println!("HSM: wifi_controller_enabled");
    let tcp_stack = net::if_up(s.spawner, s.controller, s.wifi, s.rng, s.config)
        .await
        .unwrap();

    let wifi_controller_enabled_stack = WifiControllerEnabled {
        config: s.config,
        rng: s.rng,
        uart_buf: s.uart_buf,
        tcp_stack,
    };
    match tcp_enabled(wifi_controller_enabled_stack).await {
        Ok(_) => (),
        Err(e) => {
            println!("AP Stack error: {}", e);
        }
    }
    net::ap_stack_disable().await;
    Ok(()) // todo!() return relevant value
}

pub struct TCPEnabled<'a> {
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub tcp_socket: TcpSocket<'a>,
    pub uart_buf: &'a BufferedUart,
}

cfg_if::cfg_if!(if #[cfg(feature = "esp32")] {use embassy_net::IpListenEndpoint;});
async fn tcp_enabled<'a>(s: WifiControllerEnabled<'a>) -> Result<(), sunset::Error> {
    println!("HSM: tcp_enabled");

    let mut rx_buffer = [0u8; 1536];
    let mut tx_buffer = [0u8; 1536];

    // TO FIX: ESP32 target needs tcp_socket.accept in main.rs - Gets blocked at bridge connection?
    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32")] {
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
                net::tcp_socket_disable().await;
            }
            println!("Connected, port 22");
        } else {
            let tcp_socket = net::accept_requests(s.tcp_stack, &mut rx_buffer, &mut tx_buffer).await;
        }
    );
    let tcp_enabled_struct = TCPEnabled {
        config: s.config,
        tcp_socket,
        uart_buf: s.uart_buf,
    };
    match socket_enabled(tcp_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("TCP socket error: {}", e);
        }
    }
    net::tcp_socket_disable().await;
    Ok(()) // todo!() return relevant value
}

pub struct SocketEnabled<'a> {
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: SSHServer<'a>,
    pub uart_buf: &'a BufferedUart,
}

async fn socket_enabled<'a>(s: TCPEnabled<'a>) -> Result<(), sunset::Error> {
    println!("HSM: socket_enabled");
    // loop {
    // Spawn network tasks to handle incoming connections with demo_common::session()
    let mut inbuf = [0u8; UART_BUFFER_SIZE];
    let mut outbuf = [0u8; UART_BUFFER_SIZE];
    println!("HSM: Starting ssh_server");
    let ssh_server = serve::ssh_wait_for_initialisation(&mut inbuf, &mut outbuf).await;
    println!("HSM: Started ssh_server");

    let socket_enabled_struct = SocketEnabled {
        config: s.config,
        tcp_socket: s.tcp_socket,
        ssh_server,
        uart_buf: s.uart_buf,
    };
    match ssh_enabled(socket_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("SSH server error: {}", e);
        }
    }

    serve::ssh_disable().await;
    // }
    Ok(()) // todo!() return relevant value
}

pub struct SshEnabled<'a, 'b, CL>
where
    CL: Future<Output = Result<(), sunset::Error>>,
{
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: &'b SSHServer<'a>,
    pub uart_buf: &'a BufferedUart,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub chan_pipe: &'b Channel<NoopRawMutex, serve::SessionType, 1>,
    pub connection_loop: CL,
}

async fn ssh_enabled<'a>(s: SocketEnabled<'a>) -> Result<(), sunset::Error> {
    println!("HSM: ssh_enabled");
    // loop {
    println!("HSM: Starting channel pipe");
    let chan_pipe = Channel::<NoopRawMutex, serve::SessionType, 1>::new();
    println!("HSM: Started channel pipe. Calling connection_loop from ssh_enabled");
    let connection = serve::connection_loop(&s.ssh_server, &chan_pipe, s.config);
    println!("HSM: Started connection loop");

    let ssh_enabled_struct = SshEnabled {
        tcp_socket: s.tcp_socket,
        ssh_server: &s.ssh_server,
        config: s.config,
        uart_buf: s.uart_buf,
        chan_pipe: &chan_pipe,
        connection_loop: connection,
    };
    match client_connected(ssh_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("Client connection error: {}", e);
        }
    }

    serve::connection_disable().await;
    // }
    Ok(()) // todo!() return relevant value
}

pub struct ClientConnected<'a, 'b, CL, BR>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    BR: Future<Output = Result<(), sunset::Error>>,
{
    pub ssh_server: &'b SSHServer<'a>,
    pub bridge: BR,
    pub connection_loop: CL,
    pub tcp_socket: TcpSocket<'a>,
}

async fn client_connected<'a, 'b, CL>(s: SshEnabled<'a, 'b, CL>) -> Result<(), sunset::Error>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    'a: 'b,
{
    println!("HSM: client_connected");

    // // loop {
    println!("HSM: Setting up serial bridge");
    let bridge = serve::handle_ssh_client(s.uart_buf, s.ssh_server, s.chan_pipe);

    let uart_enabled_struct = ClientConnected {
        ssh_server: s.ssh_server,
        bridge,
        connection_loop: s.connection_loop,
        tcp_socket: s.tcp_socket,
    };
    match bridge_connected(uart_enabled_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("Bridge error: {}", e);
        }
    }

    serve::bridge_disable().await;
    // }

    Ok(()) // todo!() return relevant value
}

async fn bridge_connected<'a, 'b, CL, BR>(
    s: ClientConnected<'a, 'b, CL, BR>,
) -> Result<(), sunset::Error>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    BR: Future<Output = Result<(), sunset::Error>>,
    'a: 'b,
{
    println!("HSM: bridge_connected");
    let mut tcp_socket = s.tcp_socket;
    let (mut rsock, mut wsock) = tcp_socket.split();
    println!("HSM: Running server from bridge_connected()");
    let server = s.ssh_server.run(&mut rsock, &mut wsock);
    let connection_loop = s.connection_loop;
    let bridge = s.bridge;
    println!("HSM: Main select() in bridge_connected()");
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
