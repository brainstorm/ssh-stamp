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
use embassy_futures::select::{Either4, select4};
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel};
use esp_hal::system::software_reset;

use esp_hal::{
    peripherals::UART1,
    peripherals::{SW_INTERRUPT, WIFI},
    rng::Rng,
};

cfg_if::cfg_if! {
   if #[cfg(feature = "esp32")] {
        use esp_hal::timer::timg::TimerGroup;
        use esp_hal::peripherals::{TIMG1};
   } else if #[cfg(any(feature = "esp32s2", feature = "esp32s3"))] {
       use esp_hal::peripherals::{SYSTIMER};
   } else if #[cfg(any(feature = "esp32c2"))] {
       use esp_hal::interrupt::software::SoftwareInterruptControl;
       use esp_hal::peripherals::{SYSTIMER};
   } else {
       use esp_hal::interrupt::software::SoftwareInterruptControl;
       use esp_hal::peripherals::{SYSTIMER};
   }
}

use esp_println::println;
use esp_radio::Controller;
// use ssh_stamp::espressif;
use ssh_stamp::{
    config::SSHStampConfig,
    espressif::{
        buffered_uart,
        buffered_uart::BufferedUart,
        buffered_uart::GPIOS,
        // net::accept_requests,
        net,
        rng,
    },
    serve,
};
use static_cell::StaticCell;
use storage::flash;
use sunset_async::{SSHServer, SunsetMutex};

pub async fn peripherals_wait_for_initialisation<'a>() -> SshStampPeripherals<'a> {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    println!("Initialising rng ");

    let rng = Rng::new();
    rng::register_custom_rng(rng);

    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32")] {
            let timg1 = peripherals.TIMG1;
        } else  {
            let systimer = peripherals.SYSTIMER;
        }
    );

    println!("Initialising flash ");
    // Read SSH configuration from Flash (if it exists)
    flash::init(peripherals.FLASH);

    println!("Loading config ");
    let config = {
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
    let config = CONFIG.init(SunsetMutex::new(config));

    let wifi = peripherals.WIFI;
    let uart1 = peripherals.UART1;
    let sw_interrupt = peripherals.SW_INTERRUPT;

    println!("Initialising gpio ");
    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32")] {
            let gpios = GPIOS {
            gpio1: peripherals.GPIO1,
            gpio3: peripherals.GPIO3,
            };
        } else {
            let gpios = GPIOS {
            gpio10: peripherals.GPIO10,
            gpio11: peripherals.GPIO11,
            };
        }
    );

    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32")] {
            let ssh_stamp_peripherals = SshStampPeripherals {
                rng: rng,
                wifi: wifi,
                config: config,
                gpios: gpios,
                uart1: uart1,
                timg1: timg1,
                sw_interrupt: sw_interrupt,
            };
        } else  {
            let ssh_stamp_peripherals = SshStampPeripherals {
                rng: rng,
                wifi: wifi,
                config: config,
                gpios: gpios,
                uart1: uart1,
                systimer: systimer,
                sw_interrupt: sw_interrupt,
            };
        }
    );

    ssh_stamp_peripherals
}

pub async fn peripherals_disable() -> () {
    // drop peripherals
    software_reset();
}

cfg_if::cfg_if!(
    if #[cfg(feature = "esp32")] {
        pub struct SshStampPeripherals<'a> {
            pub rng: Rng,
            pub wifi: WIFI<'a>,
            pub config: &'a SunsetMutex<SSHStampConfig>,
            pub gpios: GPIOS<'a>,
            pub uart1: UART1<'a>,
            pub timg1: TIMG1<'a>,
            pub sw_interrupt: SW_INTERRUPT<'a>,
        }
    } else  {
        pub struct SshStampPeripherals<'a> {
            pub rng: Rng,
            pub wifi: WIFI<'a>,
            pub config: &'a SunsetMutex<SSHStampConfig>,
            pub gpios: GPIOS<'a>,
            pub uart1: UART1<'a>,
            pub systimer: SYSTIMER<'a>,
            pub sw_interrupt: SW_INTERRUPT<'a>,
        }
    }
);

pub struct SshStampInit<'a> {
    pub rng: Rng,
    pub wifi: WIFI<'a>,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub gpios: GPIOS<'a>,
    pub uart1: UART1<'a>,
    pub spawner: Spawner,
}
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    println!("HSM: main");
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
    println!("HSM: Initialising peripherals ");
    let peripherals = peripherals_wait_for_initialisation().await;

    println!("HSM: Initialising timers ");
    cfg_if::cfg_if! {
       if #[cfg(feature = "esp32")] {
            // TODO: Test this feature configuration
            let timg1 = TimerGroup::new(peripherals.timg1);
            esp_rtos::start(timg1.timer0);
       } else if #[cfg(any(feature = "esp32s2", feature = "esp32s3"))] {
            // TODO: Test this feature configuration
           use esp_hal::timer::systimer::SystemTimer;
           let systimer = SystemTimer::new(peripherals.systimer);
           esp_rtos::start(systimer.alarm0);
       } else {
           let sw_int = SoftwareInterruptControl::new(peripherals.sw_interrupt);
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
        gpios: peripherals.gpios,
        uart1: peripherals.uart1,
        spawner: spawner,
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
    pub gpios: GPIOS<'a>,
    pub uart1: UART1<'a>,
    pub spawner: Spawner,
}
async fn peripherals_enabled(s: SshStampInit<'static>) -> Result<(), sunset::Error> {
    println!("HSM: peripherals_enabled");
    let controller = esp_radio::init().unwrap();

    let peripherals_enabled_struct = PeripheralsEnabled {
        rng: s.rng,
        wifi: s.wifi,
        config: s.config,
        controller: controller,
        gpios: s.gpios,
        uart1: s.uart1,
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
    pub gpios: GPIOS<'a>,
    pub uart1: UART1<'a>,
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
        gpios: s.gpios,
        uart1: s.uart1,
        tcp_stack: tcp_stack,
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
    pub gpios: GPIOS<'a>,
    pub uart1: UART1<'a>,
}

async fn tcp_enabled<'a>(s: WifiControllerEnabled<'a>) -> Result<(), sunset::Error> {
    println!("HSM: tcp_enabled");
    let mut rx_buffer = [0u8; 1536];
    let mut tx_buffer = [0u8; 1536];
    let tcp_socket = net::create_tcp_socket(s.tcp_stack, &mut rx_buffer, &mut tx_buffer).await;
    println!("Connected, port 22");
    let tcp_enabled_struct = TCPEnabled {
        config: s.config,
        tcp_socket: tcp_socket,
        gpios: s.gpios,
        uart1: s.uart1,
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
    pub gpios: GPIOS<'a>,
    pub uart1: UART1<'a>,
}

use buffered_uart::UART_BUFFER_SIZE;
async fn socket_enabled<'a>(s: TCPEnabled<'a>) -> Result<(), sunset::Error> {
    println!("HSM: socket_enabled");
    // loop {
    // Spawn network tasks to handle incoming connections with demo_common::session()
    let mut inbuf = [0u8; UART_BUFFER_SIZE];
    let mut outbuf = [0u8; UART_BUFFER_SIZE];
    println!("Starting ssh_server");
    let ssh_server = serve::ssh_wait_for_initialisation(&mut inbuf, &mut outbuf).await;
    println!("Started ssh_server");

    let socket_enabled_struct = SocketEnabled {
        config: s.config,
        tcp_socket: s.tcp_socket,
        ssh_server: ssh_server,
        gpios: s.gpios,
        uart1: s.uart1,
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
    pub uart1: UART1<'a>,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    pub chan_pipe: &'b Channel<NoopRawMutex, serve::SessionType, 1>,
    pub connection_loop: CL,
    pub gpios: GPIOS<'a>,
}

async fn ssh_enabled<'a>(s: SocketEnabled<'a>) -> Result<(), sunset::Error>
// where
    // 'b: 'a,
{
    println!("HSM: ssh_enabled");
    // loop {
    println!("HSM: Starting channel pipe");
    let chan_pipe = Channel::<NoopRawMutex, serve::SessionType, 1>::new();
    println!("HSM: Started channel pipe. Calling connection_loop from ssh_enabled");
    let connection = serve::connection_loop(&s.ssh_server, &chan_pipe, &s.config);
    println!("HSM: Started connection loop");

    let ssh_enabled_struct = SshEnabled {
        tcp_socket: s.tcp_socket,
        ssh_server: &s.ssh_server,
        config: s.config,
        chan_pipe: &chan_pipe,
        connection_loop: connection,
        uart1: s.uart1,
        gpios: s.gpios,
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
    pub gpios: GPIOS<'a>,
}
async fn client_connected<'a, 'b, CL>(s: SshEnabled<'a, 'b, CL>) -> Result<(), sunset::Error>
// where 'b: 'a {
where
    CL: Future<Output = Result<(), sunset::Error>>,
{
    println!("HSM: client_connected");
    // loop {
    let uart_buff = buffered_uart::uart_buffer_wait_for_initialisation().await;

    let client_connected_struct = ClientConnected {
        tcp_socket: s.tcp_socket,
        ssh_server: s.ssh_server,
        chan_pipe: s.chan_pipe,
        connection_loop: s.connection_loop,
        uart_buff: uart_buff,
        uart1: s.uart1,
        config: s.config,
        gpios: s.gpios,
    };
    match uart_buffer_ready(client_connected_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("UART buffer error: {}", e);
        }
    }

    buffered_uart::uart_buffer_disable().await;
    // }
    Ok(()) // todo!() return relevant value
}

pub struct UartBufferReady<'a, 'b, CL, U>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    U: Future<Output = Result<(), sunset::Error>>,
{
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: &'b SSHServer<'a>,
    pub uart_buff: &'a BufferedUart,
    pub chan_pipe: &'b Channel<NoopRawMutex, serve::SessionType, 1>,
    pub connection_loop: CL,
    pub uart: U,
}

async fn uart_buffer_ready<'a, 'b, CL>(s: ClientConnected<'a, 'b, CL>) -> Result<(), sunset::Error>
where
    // 'b: 'a,
    CL: Future<Output = Result<(), sunset::Error>>,
{
    println!("HSM: uart_buffer_ready");
    // loop {
    let uart = buffered_uart::uart_task(s.uart_buff, s.uart1, &s.config, s.gpios);

    let uart_buffer_ready_struct = UartBufferReady {
        tcp_socket: s.tcp_socket,
        ssh_server: s.ssh_server,
        chan_pipe: s.chan_pipe,
        uart_buff: s.uart_buff,
        connection_loop: s.connection_loop,
        uart: uart,
    };
    match uart_enabled(uart_buffer_ready_struct).await {
        Ok(_) => (),
        Err(e) => {
            println!("UART error: {}", e);
        }
    }

    buffered_uart::uart_disable().await;
    // }
    Ok(()) // todo!() return relevant value
}

pub struct UartEnabled<'a, 'b, CL, U, BR>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    U: Future<Output = Result<(), sunset::Error>>,
    BR: Future<Output = Result<(), sunset::Error>>,
{
    pub ssh_server: &'b SSHServer<'a>,
    pub bridge: BR,
    pub connection_loop: CL,
    pub uart: U,
    pub tcp_socket: TcpSocket<'a>,
}

async fn uart_enabled<'a, 'b, CL, U>(s: UartBufferReady<'a, 'b, CL, U>) -> Result<(), sunset::Error>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    U: Future<Output = Result<(), sunset::Error>>,
    'a: 'b,
{
    println!("HSM: uart_enabled");
    // // loop {
    println!("Setting up serial bridge");
    let bridge = serve::bridge_wait_for_initialisation(s.uart_buff, s.ssh_server, s.chan_pipe);

    let uart_enabled_struct = UartEnabled {
        ssh_server: s.ssh_server,
        bridge: bridge,
        connection_loop: s.connection_loop,
        uart: s.uart,
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

async fn bridge_connected<'a, 'b, CL, U, BR>(
    s: UartEnabled<'a, 'b, CL, U, BR>,
) -> Result<(), sunset::Error>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    U: Future<Output = Result<(), sunset::Error>>,
    BR: Future<Output = Result<(), sunset::Error>>,
    'a: 'b,
{
    println!("HSM: bridge_connected");
    let mut tcp_socket = s.tcp_socket;
    let (mut rsock, mut wsock) = tcp_socket.split();
    println!("Running server from handle_ssh_client()");
    let server = s.ssh_server.run(&mut rsock, &mut wsock);
    let connection_loop = s.connection_loop;
    let bridge = s.bridge;
    println!("Main select() in bridge_connected()");
    match select4(server, connection_loop, bridge, s.uart).await {
        Either4::First(r) => r,
        Either4::Second(r) => r,
        Either4::Third(r) => r,
        Either4::Fourth(r) => r,
    }?;
    Result::Ok(())
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
