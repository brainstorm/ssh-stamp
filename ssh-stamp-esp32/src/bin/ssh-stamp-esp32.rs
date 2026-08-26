// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! ESP32-family `ssh-stamp` binary.
//!
//! Brings up ESP-specific peripherals (heap, flash, RNG, UART, radio), then
//! hands control to the platform-agnostic [`ssh_stamp::app::run_app`].
//!
//! # Pin assignments
//!
//! UART, CAN and I2C pin numbers are defined per-board in `boards/*.toml`
//! files in the `ssh-stamp-esp32-boards` crate. Select a board via a
//! `board-<name>` feature (e.g. `board-esp32c6-devkitc`). That crate's front
//! page carries the generated catalog: which GPIO each bus uses on each
//! board, and which buses a board does not support yet.

#![no_std]
#![no_main]

extern crate alloc;

use embassy_executor::Spawner;
use esp_println::logger;
use heapless::String;
use log::{debug, error, warn};
use ssh_stamp::config::{SSHStampConfig, UartPins};
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::store;
use ssh_stamp::{
    app,
    mem_probe::{self, Checkpoint},
    settings::DEFAULT_IP,
};
#[cfg(feature = "can")]
use ssh_stamp_esp32::{BufferedCan, CAN_BUF, EspCanPins, can_task};
use ssh_stamp_esp32::{
    EspPlatform, EspUartPins, EspWifi, bench, entropy_source_active, flash, mac_address,
    spawn_uart, start_interrupt_executor,
};
use ssh_stamp_esp32_boards::Board;
use ssh_stamp_hal::{HalError, WifiError};
use ssh_stamp_hal::{NetworkProviderHal, WifiHal};
use static_cell::StaticCell;
use sunset_async::SunsetMutex;

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_bootloader_esp_idf::esp_app_desc!();
    logger::init_logger_from_env();

    debug!("HSM: initialising peripherals");
    ssh_stamp_esp32::boot!(peripherals, rng, entropy_source, sw_int1);

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

    // On first boot this mints the SSH host key and the WiFi PSK, so the
    // entropy source enabled above has to still be running. Guard the
    // invariant rather than trusting a comment: `debug-assertions` are on
    // even in release for this workspace, so reintroducing an early drop of
    // the `EntropySource` fails loudly on the bench instead of silently
    // producing predictable keys.
    debug_assert!(
        entropy_source_active(),
        "entropy source was disabled before host key generation"
    );

    debug!("Loading config");
    let flash_config = {
        let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
            panic!("Could not acquire flash storage lock");
        };
        let mut fb = flash_storage_guard.lock().await;
        let (flash_storage, buf) = fb.split_ref_mut();
        store::load_or_create(flash_storage, buf, mac_address(), uart_pins)
    }
    // Deliberately fatal. `load_or_create` only errors when a config *is*
    // present but fails its version or integrity check; recreating one there
    // would regenerate the host key (breaking client host-key pinning) and
    // reopen the unauthenticated first-login window. Refusing to boot is the
    // safe side of that trade. Recover by erasing the config sector, e.g.
    // `espflash erase-region 0x9000 0x1000`, which makes the next boot mint a
    // fresh config.
    .expect(
        "Stored config is present but invalid; refusing to overwrite it. \
         Erase the config sector (espflash erase-region 0x9000 0x1000) to reprovision.",
    );

    // Line settings for the bridge; the UART task is configured with them
    // below, so `SSH_STAMP_UART_*` changes take effect on the next boot.
    let uart_params = flash_config.uart_params;

    static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();
    let config: &'static SunsetMutex<SSHStampConfig> = CONFIG.init(SunsetMutex::new(flash_config));

    mem_probe::checkpoint(Checkpoint::Boot);
    let interrupt_spawner = start_interrupt_executor(sw_int1);
    let uart_buf = spawn_uart(interrupt_spawner, peripherals.UART1, pins, uart_params);

    #[cfg(feature = "can")]
    let can_buf: &'static BufferedCan = {
        // Boards that mux their CAN pins with other functions declare the
        // routing in their TOML; a no-op for boards without a [can_mux].
        ssh_stamp_esp32_boards::setup_can_transceiver!(peripherals);

        let can_pins = ssh_stamp_esp32_boards::take_can_pins!(peripherals);
        let can_pins = EspCanPins {
            tx: can_pins.0,
            rx: can_pins.1,
        };
        let can_buf = CAN_BUF.init_with(BufferedCan::new);
        interrupt_spawner
            .spawn(can_task(can_buf, peripherals.TWAI0, can_pins).expect("can_task spawn failed"));
        can_buf
    };

    #[cfg(feature = "can")]
    let platform = EspPlatform::new(can_buf);
    #[cfg(not(feature = "can"))]
    let platform = EspPlatform::new();

    mem_probe::checkpoint(Checkpoint::PeripheralsReady);
    bench::log_heap("peripherals");

    debug!("Initialising radio");

    // Last consumer of randomness before the radio: mints the WiFi PSK if the
    // config did not already carry one.
    let ap_config = app::prepare_ap_config(config, &platform)
        .await
        .expect("Failed to prepare AP config");

    // Hand the entropy source over to the radio. The `WifiController::new`
    // inside `bring_up()` enables the RF subsystem, which is itself an
    // entropy source, and Espressif requires the ADC source to be off before
    // the RF subsystem starts. Nothing is lost by dropping it here, the
    // key-exchange material sunset draws per connection is covered by the
    // radio from now on.
    drop(entropy_source);

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

    mem_probe::checkpoint(Checkpoint::WifiUp);
    bench::log_heap("wifi_up");

    // The radio should have picked up the entropy duty dropped above:
    // esp-radio bumps esp-hal's entropy-source count once the RF subsystem is
    // running. sunset draws fresh key-exchange material from `getrandom` for
    // every SSH connection served below, so if this does not hold the handover
    // has a hole in it.
    debug_assert!(
        entropy_source_active(),
        "no entropy source active after WiFi came up"
    );

    if let Err(e) = app::run_app(stack.unwrap(), uart_buf, config, &platform).await {
        error!("run_app exited with error: {e}");
    }

    warn!("End of main, resetting");
    esp_hal::system::software_reset();
}

// The `getrandom` custom backend.
ssh_stamp_esp32::getrandom_backend!();

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
