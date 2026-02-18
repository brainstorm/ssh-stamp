#![no_std]
#![no_main]

// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::marker::Sized;
use esp_alloc as _;
use esp_backtrace as _;

#[cfg(feature = "esp32")]
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{
    gpio::Pin,
    interrupt::{Priority, software::SoftwareInterruptControl},
    peripherals::UART1,
    rng::Rng,
    uart::{Config, RxConfig, Uart},
};
use esp_println::dbg;
use esp_rtos::embassy::InterruptExecutor;
use esp_storage::FlashStorage;

use embassy_executor::Spawner;

use log::info;

use ssh_stamp::pins;
use ssh_stamp::pins::GPIOConfig;
use ssh_stamp::pins::PinChannel;
use ssh_stamp::{
    config::SSHStampConfig,
    espressif::{
        buffered_uart::BufferedUart,
        net::{accept_requests, if_up},
        rng,
    },
    storage::Fl,
};

use static_cell::StaticCell;
use sunset_async::SunsetMutex;

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
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

    // System init
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut rng = Rng::new();
    // let timg0 = TimerGroup::new(peripherals.TIMG0); dhcp example uses it for esp_rtos as timg0.timer0

    rng::register_custom_rng(rng);
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

    // TODO: Migrate this function/test to embedded-test.
    // Quick roundtrip test for SSHStampConfig
    // ssh_stamp::config::roundtrip_config();

    // Read SSH configuration from Flash (if it exists)
    let mut flash_storage = Fl::new(FlashStorage::new(peripherals.FLASH));
    let config = ssh_stamp::storage::load_or_create(&mut flash_storage).await;

    static FLASH: StaticCell<SunsetMutex<Fl>> = StaticCell::new();
    let _flash = FLASH.init(SunsetMutex::new(flash_storage));

    static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();
    let config = CONFIG.init(SunsetMutex::new(config.unwrap()));

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

    let serde_pin_config = {
        let guard = config.lock().await;
        guard.uart_pins.clone()
    };

    cfg_if::cfg_if! {
        if #[cfg(any(feature = "esp32c2", feature = "esp32c3"))] {
           // TODO: ESPC2/C3: GPIO pin configuration not yet implemented
           let available_gpios = GPIOConfig {
               gpio10: Some(peripherals.GPIO10.degrade()),
               gpio11: Some(peripherals.GPIO9.degrade()),
            };
        }else if #[cfg(feature = "esp32c6")] {

            let available_gpios = GPIOConfig {
                gpio10: Some(peripherals.GPIO10.degrade()),
                gpio11: Some(peripherals.GPIO11.degrade()),
            };
        } else if #[cfg(feature = "esp32")]{
            // TODO: ESP32: GPIO pin configuration not yet implemented
            let available_gpios = GPIOConfig {
                gpio10: Some(peripherals.GPIO12.degrade()),
                gpio11: Some(peripherals.GPIO13.degrade()),
            };
        } else if #[cfg(any( feature = "esp32s2", feature = "esp32s3"))]{
            // TODO: ESP32/S2/S3: GPIO pin configuration not yet implemented
            let available_gpios = GPIOConfig {
                gpio10: Some(peripherals.GPIO10.degrade()),
                gpio11: Some(peripherals.GPIO11.degrade()),
            };
        } else {
            // No fallback
            panic!("Unsupported target for PinChannel GPIOConfig initialization");
        }
    };

    // Initialize the global pin channel and keep the &'static reference so we can
    // pass it to tasks that need to mutate pins (no unsafe globals).
    let pin_channel_ref =
        pins::init_global_channel(PinChannel::new(serde_pin_config, available_gpios));

    // Grab UART1, typically not connected to dev board's TTL2USB IC nor builtin JTAG functionality
    let uart1 = peripherals.UART1;

    // Use the same config reference for UART task.
    // Pass pin_channel_ref into the UART task so it can acquire/release pins.
    interrupt_spawner
        .spawn(uart_task(uart_buf, uart1, pin_channel_ref))
        .unwrap();
    // Optional version details logging
    if let Some(build_date) = option_env!("BUILD_DATE") {
        info!(
            "Initialization done. Starting SSH server... version : v{}, build date: {}",
            env!("CARGO_PKG_VERSION"),
            build_date
        );
    }

    // Pass pin_channel_ref into accept_requests (so SSH handlers can use it).
    // NOTE: accept_requests signature must accept this arg; if it doesn't,
    // thread the reference into whatever code spawns handle_ssh_client.
    accept_requests(tcp_stack, uart_buf, pin_channel_ref).await;
}

static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();
static INT_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new(); // 0 is used for esp_rtos.

#[embassy_executor::task()]
async fn uart_task(
    buffer: &'static BufferedUart,
    uart_periph: UART1<'static>,
    pin_channel_ref: &'static SunsetMutex<PinChannel>,
) {
    dbg!("Spawning UART task...");
    // Hardware UART setup
    let uart_config = Config::default().with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1),
    );

    // Use the pinned reference passed in from main.
    let mut pin_chan = pin_channel_ref.lock().await;

    // Sync pin config via channels
    pin_chan
        .with_channel(async |rx, tx| {
            let uart = Uart::new(uart_periph, uart_config)
                .unwrap()
                .with_rx(rx)
                .with_tx(tx)
                .into_async();

            // Run the main buffered TX/RX loop
            buffer.run(uart).await;
        })
        .await
        .unwrap();
}
