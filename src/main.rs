#![no_std]
#![no_main]

// SPDX-FileCopyrightText: 2025 Julio Beltran Ortega, Anthony Tambasco, Roman Valls Guimera, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::marker::Sized;
use esp_alloc as _;
use esp_backtrace as _;
#[cfg(feature = "esp32")]
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{
    gpio::AnyPin,
    interrupt::{Priority, software::SoftwareInterruptControl},
    peripherals::UART1,
    rng::Rng,
    uart::{Config, RxConfig, Uart},
};
use embassy_executor::Spawner;
use esp_rtos::embassy::InterruptExecutor;

use ssh_stamp::config::SSHStampConfig;
use ssh_stamp::espressif::{
    buffered_uart::BufferedUart,
    net::{accept_requests, if_up},
    rng,
};

use storage::flash;

use static_cell::StaticCell;
use sunset_async::SunsetMutex;

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
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

    // System init
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut rng = Rng::new();
    
    rng::register_custom_rng(rng);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    cfg_if::cfg_if! {
       if #[cfg(feature = "esp32")] {
            let timg1 = TimerGroup::new(peripherals.TIMG1);
            esp_rtos::start(timg1.timer0, sw_int);
       } else {
           use esp_hal::timer::systimer::SystemTimer;
           let systimer = SystemTimer::new(peripherals.SYSTIMER);
           esp_rtos::start(systimer.alarm0, sw_int.software_interrupt0);
       }
    }
    
    flash::init(peripherals.FLASH);

    let config = {
        // let rrr = flash::get_flash_n_buffer();
        let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
            panic!("Could not acquire flash storage lock");
        };
        let mut flash_storage = flash_storage_guard.lock().await;
        // TODO: Migrate this function/test to embedded-test.
        // Quick roundtrip test for SSHStampConfig
        // ssh_stamp::config::roundtrip_config();
        ssh_stamp::storage::load_or_create(&mut flash_storage).await
    }
    .expect("Could not load or create SSHStampConfig");

    static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();
    let config = CONFIG.init(SunsetMutex::new(config));
    
    let wifi_controller = esp_radio::init().unwrap();

    // Bring up the network interface and start accepting SSH connections.
    // Clone the reference to config to avoid borrow checker issues.
    let tcp_stack = if_up(spawner, wifi_controller, peripherals.WIFI, &mut rng, config)
        .await 
        .unwrap();

    // Set up software buffered UART to run in a higher priority InterruptExecutor
    let uart_buf = UART_BUF.init_with(BufferedUart::new);
    let interrupt_executor =
        INT_EXECUTOR.init_with(|| InterruptExecutor::new(sw_int.software_interrupt1));
    cfg_if::cfg_if! {
        if #[cfg(any(feature = "esp32", feature = "esp32s2", feature = "esp32s3"))] {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority1);
        } else {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority10);
        }
    }
    cfg_if::cfg_if! {
        if #[cfg(not(feature = "esp32c2"))] {
    interrupt_spawner.spawn(uart_task(uart_buf, peripherals.UART1, peripherals.GPIO11.into(), peripherals.GPIO10.into())).unwrap();
        } else {
            interrupt_spawner.spawn(uart_task(uart_buf, peripherals.UART1, peripherals.GPIO9.into(), peripherals.GPIO10.into())).unwrap();
        }
    }
    accept_requests(tcp_stack, uart_buf).await;
}

static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();
static INT_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new(); // 0 is used for esp_rtos

#[embassy_executor::task]
async fn uart_task(
    buffer: &'static BufferedUart,
    uart_periph: UART1<'static>,
    rx_pin: AnyPin<'static>,
    tx_pin: AnyPin<'static>,
) {
    // Hardware UART setup
    let uart_config = Config::default().with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1),
    );

    let uart = Uart::new(uart_periph, uart_config)
        .unwrap()
        .with_rx(rx_pin)
        .with_tx(tx_pin)
        .into_async();

    // Run the main buffered TX/RX loop
    buffer.run(uart).await;
}
