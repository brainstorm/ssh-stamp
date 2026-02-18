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
use embassy_net::IpListenEndpoint;
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel};
use esp_hal::gpio::AnyPin;
use esp_hal::system::software_reset;
use esp_rtos::embassy::InterruptExecutor;

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
       // use esp_hal::interrupt::software::SoftwareInterruptControl;
       use esp_hal::peripherals::{SYSTIMER};
   } else {
       // use esp_hal::interrupt::software::SoftwareInterruptControl;
       use esp_hal::peripherals::{SYSTIMER};
   }
}
use esp_println::println;
use esp_radio::Controller;
use ssh_stamp::{
    config::SSHStampConfig,
    espressif::{
        buffered_uart,
        buffered_uart::BufferedUart,
        // buffered_uart::GPIOS,
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
            gpio13: peripherals.GPIO13.into(),
            gpio14: peripherals.GPIO14.into(),
            };
        } else {
            let gpios = GPIOS {
            gpio10: peripherals.GPIO10.into(),
            gpio11: peripherals.GPIO11.into(),
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
// use esp_hal::interrupt::software::SoftwareInterrupt;
use esp_hal::interrupt::{Priority, software::SoftwareInterruptControl};
pub struct SshStampInit<'a> {
    pub rng: Rng,
    pub wifi: WIFI<'a>,
    pub config: &'a SunsetMutex<SSHStampConfig>,
    // pub gpios: GPIOS<'a>,
    pub uart_buf: &'a BufferedUart,
    pub spawner: Spawner,
}
static INT_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new(); // 0 is used for esp_rtos
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
    let software_interrupts = SoftwareInterruptControl::new(peripherals.sw_interrupt);

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
           // let sw_int = SoftwareInterruptControl::new(peripherals.sw_interrupt);
           use esp_hal::timer::systimer::SystemTimer;
           let systimer = SystemTimer::new(peripherals.systimer);
           esp_rtos::start(systimer.alarm0, software_interrupts.software_interrupt0);
       }
    }

    // Set up software buffered UART to run in a higher priority InterruptExecutor
    // Must be higher priority than esp_rtos (Priority1)
    let uart_buf = UART_BUF.init_with(BufferedUart::new);
    let interrupt_executor =
        INT_EXECUTOR.init_with(|| InterruptExecutor::new(software_interrupts.software_interrupt1));
    cfg_if::cfg_if! {
        if #[cfg(any(feature = "esp32", feature = "esp32s2", feature = "esp32s3"))] {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority3);
        } else {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority10);
        }
    }

    // Use the same config reference for UART task.
    // Pass pin_channel_ref into the UART task so it can acquire/release pins.
    cfg_if::cfg_if! {
       if #[cfg(feature = "esp32")] {
        interrupt_spawner
            .spawn(uart_task(
                uart_buf,
                peripherals.uart1,
                &peripherals.config,
                // peripherals.gpios,
                peripherals.gpios.gpio13.into(),
                peripherals.gpios.gpio14.into(),
                ))
                .unwrap();
       } else {
           interrupt_spawner
               .spawn(uart_task(
                   uart_buf,
                   peripherals.uart1,
                   &peripherals.config,
                   // peripherals.gpios,
                   peripherals.gpios.gpio10.into(),
                   peripherals.gpios.gpio11.into(),
               ))
               .unwrap();
       }
    }
    // TODO: Migrate this function/test to embedded-test.
    // Quick roundtrip test for SSHStampConfig
    // ssh_stamp::config::roundtrip_config();

    let peripherals_enabled_struct = SshStampInit {
        rng: peripherals.rng,
        wifi: peripherals.wifi,
        config: peripherals.config,
        spawner: spawner,
        uart_buf: uart_buf,
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
        controller: controller,
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
    pub uart_buf: &'a BufferedUart,
}

async fn tcp_enabled<'a>(s: WifiControllerEnabled<'a>) -> Result<(), sunset::Error> {
    println!("HSM: tcp_enabled");
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
        net::tcp_socket_disable().await;
    }
    // let tcp_socket = net::create_tcp_socket(s.tcp_stack, &mut rx_buffer, &mut tx_buffer);
    println!("Connected, port 22");
    let tcp_enabled_struct = TCPEnabled {
        config: s.config,
        tcp_socket: tcp_socket,
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

use buffered_uart::UART_BUFFER_SIZE;
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
        ssh_server: ssh_server,
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
use embassy_time::{Duration, Timer};
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
    Timer::after(Duration::from_millis(500)).await;
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

pub struct ClientConnected<'a, 'b, CL>
where
    CL: Future<Output = Result<(), sunset::Error>>,
{
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: &'b SSHServer<'a>,
    pub chan_pipe: &'b Channel<NoopRawMutex, serve::SessionType, 1>,
    pub connection_loop: CL,
    pub uart_buf: &'a BufferedUart,
    pub config: &'a SunsetMutex<SSHStampConfig>,
}
async fn client_connected<'a, 'b, CL>(s: SshEnabled<'a, 'b, CL>) -> Result<(), sunset::Error>
// where 'b: 'a {
where
    CL: Future<Output = Result<(), sunset::Error>>,
{
    println!("HSM: client_connected");
    // loop {
    // let uart_buff = buffered_uart::uart_buffer_wait_for_initialisation().await;
    Timer::after(Duration::from_millis(500)).await;

    let client_connected_struct = ClientConnected {
        tcp_socket: s.tcp_socket,
        ssh_server: s.ssh_server,
        chan_pipe: s.chan_pipe,
        connection_loop: s.connection_loop,
        uart_buf: s.uart_buf,
        config: s.config,
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

pub struct UartBufferReady<'a, 'b, CL>
where
    CL: Future<Output = Result<(), sunset::Error>>,
{
    pub tcp_socket: TcpSocket<'a>,
    pub ssh_server: &'b SSHServer<'a>,
    pub uart_buf: &'a BufferedUart,
    pub chan_pipe: &'b Channel<NoopRawMutex, serve::SessionType, 1>,
    pub connection_loop: CL,
}

async fn uart_buffer_ready<'a, 'b, CL>(s: ClientConnected<'a, 'b, CL>) -> Result<(), sunset::Error>
where
    // 'b: 'a,
    CL: Future<Output = Result<(), sunset::Error>>,
{
    println!("HSM: uart_buffer_ready");
    Timer::after(Duration::from_millis(500)).await;

    // loop {
    // let uart = buffered_uart::uart_task(s.uart_buff, s.uart1, &s.config, s.gpios);
    // Signal for uart task to configure pins and run.
    UART_SIGNAL.signal(1);
    Timer::after(Duration::from_millis(500)).await;
    println!("HSM: UART_SIGNAL sent");

    let uart_buffer_ready_struct = UartBufferReady {
        tcp_socket: s.tcp_socket,
        ssh_server: s.ssh_server,
        chan_pipe: s.chan_pipe,
        uart_buf: s.uart_buf,
        connection_loop: s.connection_loop,
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
    println!("HSM: uart_enabled");
    Timer::after(Duration::from_millis(500)).await;

    // // loop {
    println!("HSM: Setting up serial bridge");
    let bridge = serve::bridge_wait_for_initialisation(s.uart_buf, s.ssh_server, s.chan_pipe);

    let uart_enabled_struct = UartEnabled {
        ssh_server: s.ssh_server,
        bridge: bridge,
        connection_loop: s.connection_loop,
        // uart: s.uart,
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
    s: UartEnabled<'a, 'b, CL, BR>,
) -> Result<(), sunset::Error>
where
    CL: Future<Output = Result<(), sunset::Error>>,
    BR: Future<Output = Result<(), sunset::Error>>,
    'a: 'b,
{
    println!("HSM: bridge_connected");
    let mut tcp_socket = s.tcp_socket;
    let (mut rsock, mut wsock) = tcp_socket.split();
    println!("HSM: Running server from handle_ssh_client()");
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

use esp_hal::uart::{RxConfig, Uart};

// cfg_if::cfg_if! {
//    if #[cfg(feature = "esp32")] {
//         use esp_hal::peripherals::{GPIO1, GPIO3};
//    } else {
//        use esp_hal::peripherals::{GPIO10,  GPIO11};
//    }
// }
cfg_if::cfg_if!(
    if #[cfg(feature = "esp32")] {
        pub struct GPIOS<'a> {
            pub gpio13: AnyPin<'a>,
            pub gpio14: AnyPin<'a>,
        }
    } else {
        pub struct GPIOS<'a> {
            pub gpio10: AnyPin<'a>,
            pub gpio11: AnyPin<'a>,
        }
    }
);

pub static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();

pub async fn uart_buffer_wait_for_initialisation() -> &'static BufferedUart {
    UART_BUF.init_with(BufferedUart::new)
}

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
pub static UART_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();
// use embassy_time::{Duration, Timer};
// use esp_hal::gpio::AnyPin;
use esp_println::dbg;
#[embassy_executor::task]
pub async fn uart_task(
    uart_buf: &'static BufferedUart,
    uart1: UART1<'static>,
    _config: &'static SunsetMutex<SSHStampConfig>,
    // gpios: GPIOS<'static>,
    rx_pin: AnyPin<'static>,
    tx_pin: AnyPin<'static>,
) -> () {
    dbg!("Configuring UART");
    Timer::after(Duration::from_millis(100)).await;
    // Wait until ssh complete
    UART_SIGNAL.wait().await;
    dbg!("UART signal recieved");
    Timer::after(Duration::from_millis(100)).await;

    // let config_lock = config.lock().await;
    // let rx: u8 = config_lock.uart_pins.rx;
    // let tx: u8 = config_lock.uart_pins.tx;
    // if rx != tx {
    // cfg_if::cfg_if!(
    //     if #[cfg(feature = "esp32")] {
    //         let tx_pin: AnyPin<'static> = gpio1.into();
    //         let rx_pin: AnyPin<'static> = gpio3.into();
    //         // let mut holder0 = Some(gpios.gpio1);
    //         // let mut holder1 = Some(gpios.gpio3);
    //     } else {
    //         let tx_pin = gpio10;
    //         let rx_pin = gpio11;
    //         // let mut holder0 = Some(gpios.gpio10);
    //         // let mut holder1 = Some(gpios.gpio11);
    //     }
    // );

    // let rx_pin = match rx {
    //     1 => holder0.take().unwrap().degrade(),
    //     3 => holder1.take().unwrap().degrade(),
    //     10 => holder0.take().unwrap().degrade(),
    //     11 => holder1.take().unwrap().degrade(),
    //     _ => holder0.take().unwrap().degrade(),
    // };
    // let tx_pin = match tx {
    //     1 => holder0.take().unwrap().degrade(),
    //     3 => holder1.take().unwrap().degrade(),
    //     10 => holder0.take().unwrap().degrade(),
    //     11 => holder1.take().unwrap().degrade(),
    //     _ => holder1.take().unwrap().degrade(),
    // };

    dbg!("UART config");
    Timer::after(Duration::from_millis(100)).await;

    // Hardware UART setup
    let uart_config = esp_hal::uart::Config::default().with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1),
    );

    // let uart_config = Config::default().with_rx(
    //     RxConfig::default()
    //         .with_fifo_full_threshold(16)
    //         .with_timeout(1),
    // );

    dbg!("UART setup pins");
    Timer::after(Duration::from_millis(100)).await;

    let uart = Uart::new(uart1, uart_config)
        .unwrap()
        .with_rx(rx_pin)
        .with_tx(tx_pin)
        .into_async();

    // Run the main buffered TX/RX loop
    dbg!("uart_task running UART");
    Timer::after(Duration::from_millis(100)).await;
    uart_buf.run(uart).await;
    // }
    // TODO: Pin config error
    // Ok(())
}
