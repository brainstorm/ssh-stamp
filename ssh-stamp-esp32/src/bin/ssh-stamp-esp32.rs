// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! ESP32-family `ssh-stamp` binary.
//!
//! Brings up ESP-specific peripherals (heap, flash, RNG, UART, radio), then
//! hands control to the platform-agnostic [`ssh_stamp::app::run_app`].
//!
//! # UART Pin Assignments
//!
//! UART pin numbers are defined per-board in `boards/*.toml` files in the
//! `ssh-stamp-esp32-boards` crate. Select a board via a `board-<name>` feature
//! (e.g. `board-esp32c6-devkitc`). See the `ssh-stamp-esp32-boards` crate
//! documentation for the full list.

#![no_std]
#![no_main]

extern crate alloc;

use embassy_executor::Spawner;
use esp_hal::interrupt::{Priority, software::SoftwareInterruptControl};
use esp_hal::rng::{Trng, TrngSource};
use esp_println::logger;
use esp_rtos::embassy::InterruptExecutor;
use heapless::String;
use log::{debug, error, warn};
use ssh_stamp::config::{SSHStampConfig, UartPins};
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::store;
use ssh_stamp::{app, settings::DEFAULT_IP};
use ssh_stamp_esp32::{
    BufferedUart, EspPlatform, EspUartPins, EspWifi, UART_BUF, flash, mac_address,
    register_custom_rng, uart_task,
};
use ssh_stamp_esp32_boards::Board;
use ssh_stamp_hal::{HalError, WifiError};
use ssh_stamp_hal::{NetworkProviderHal, WifiHal};
use static_cell::StaticCell;
use sunset_async::SunsetMutex;

cfg_if::cfg_if! {
    if #[cfg(feature = "esp32")] {
        use esp_hal::timer::timg::TimerGroup;
    }
}

static INT_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new(); // 0 is used for esp_rtos

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32s2")] {
            // TODO: This heap size will crash at runtime (only for the ESP32S2);
            // see https://github.com/brainstorm/ssh-stamp/pull/41#issuecomment-2964775170
            esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 72 * 1024);
        } else {
            esp_alloc::heap_allocator!(size: 72 * 1024);
        }
    );
    esp_bootloader_esp_idf::esp_app_desc!();
    logger::init_logger_from_env();
    debug!("HSM: initialising peripherals");

    let peripherals = esp_hal::init(esp_hal::Config::default());

    // Enable true random number generation using ADC entropy source before config creation.
    // The ESP32 hardware RNG only produces true random numbers when RF subsystem is enabled
    // OR ADC entropy source is active. This ensures WiFi password and SSH hostkey use
    // cryptographically secure random values.
    // See: https://github.com/brainstorm/ssh-stamp/issues/10
    // See: https://github.com/esp-rs/esp-hal/pull/3829
    let trng_source = TrngSource::new(peripherals.RNG, peripherals.ADC1);
    let trng = Trng::try_new().unwrap();
    let rng = trng.downgrade();
    register_custom_rng(rng);
    drop(trng_source);

    debug!("Initialising flash");
    flash::init(peripherals.FLASH);

    #[cfg(feature = "sftp-ota")]
    {
        use ssh_stamp_hal::OtaActions;
        ssh_stamp_esp32::EspOtaWriter::try_validating_current_ota_partition()
            .await
            .expect("Failed to validate the current ota partition");
    }

    // Board selection — the generated select_board! macro expands to a
    // cfg_if! that imports the active board's struct as B. The pin numbers
    // come from boards/*.toml via build.rs codegen — no per-board lines here.
    ssh_stamp_esp32_boards::select_board!();
    debug!("Active board: {}", B::NAME);

    let (rx_pin, tx_pin, rx_num, tx_num) = ssh_stamp_esp32_boards::take_uart_pins!(peripherals);
    let pins = EspUartPins {
        rx: rx_pin,
        tx: tx_pin,
    };
    let uart_pins = UartPins {
        rx: rx_num,
        tx: tx_num,
    };

    debug!("Loading config");
    let flash_config = {
        let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
            panic!("Could not acquire flash storage lock");
        };
        let mut fb = flash_storage_guard.lock().await;
        let (flash_storage, buf) = fb.split_ref_mut();
        store::load_or_create(flash_storage, buf, mac_address(), uart_pins)
    }
    .expect("Could not load or create SSHStampConfig");

    static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();
    let config: &'static SunsetMutex<SSHStampConfig> = CONFIG.init(SunsetMutex::new(flash_config));

    debug!("Initialising timers");
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    cfg_if::cfg_if! {
       if #[cfg(feature = "esp32")] {
            // TODO: Test this feature configuration
            let timg1 = TimerGroup::new(peripherals.TIMG1);
             esp_rtos::start(timg1.timer0, sw_int.software_interrupt0);
       } else if #[cfg(any(feature = "esp32s2", feature = "esp32s3"))] {
            // TODO: Test this feature configuration
           use esp_hal::timer::systimer::SystemTimer;
           let systimer = SystemTimer::new(peripherals.SYSTIMER);
            esp_rtos::start(systimer.alarm0, sw_int.software_interrupt0);
       } else {
           use esp_hal::timer::systimer::SystemTimer;
           let systimer = SystemTimer::new(peripherals.SYSTIMER);
           esp_rtos::start(systimer.alarm0, sw_int.software_interrupt0);
       }
    }

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
    interrupt_spawner
        .spawn(uart_task(uart_buf, peripherals.UART1, pins).expect("uart_task spawn failed"));

    debug!("Initialising radio");

    let platform = EspPlatform::new();
    let ap_config = app::prepare_ap_config(config, &platform)
        .await
        .expect("Failed to prepare AP config");

    let mut wifi = EspWifi::new(spawner, peripherals.WIFI, rng, DEFAULT_IP);
    wifi.configure_ap(ap_config)
        .expect("Failed to configure AP");

    let stack = wifi.bring_up().await;
    match stack {
        Ok(_) => (),
        Err(ref e) => {
            warn!("Failed to bring up WiFi");
            if let HalError::Wifi(WifiError::StationMode) = e {
                let mut config_guard = config.lock().await;
                config_guard.wifi_sta_ssid = String::<32>::new();
                let _ = platform.save_config(&config_guard).await;
                warn!("Station Mode failed to connect. Rebooting into Access Point mode...");
                platform.reset();
            }
        }
    }

    if let Err(e) = app::run_app(stack.unwrap(), uart_buf, config, &platform).await {
        error!("run_app exited with error: {e}");
    }

    warn!("End of main, resetting");
    esp_hal::system::software_reset();
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
