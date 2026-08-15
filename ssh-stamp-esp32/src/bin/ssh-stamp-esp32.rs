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
#[cfg(not(any(feature = "esp32c5", feature = "esp32c61")))]
use esp_hal::rng::{Trng, TrngSource};
use esp_println::logger;
use esp_rtos::embassy::InterruptExecutor;
use heapless::String;
use log::{debug, error, warn};
use ssh_stamp::config::{SSHStampConfig, UartPins};
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::store;
use ssh_stamp::{app, settings::DEFAULT_IP};
#[cfg(feature = "can")]
use ssh_stamp_esp32::{BufferedCan, CAN_BUF, EspCanPins, can_task};
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
    //
    // TODO: The ESP32-C5/C61 TRNG is not yet available in esp-hal 1.1.1. Once
    // https://github.com/esp-rs/esp-hal/pull/4978 lands in a release, remove
    // this cfg_if! and use Trng/TrngSource unconditionally for all targets.
    cfg_if::cfg_if! {
        if #[cfg(any(feature = "esp32c5", feature = "esp32c61"))] {
            // ESP32-C5/C61 have no TRNG peripheral — use the basic Rng directly.
            // Until the TODO above is resolved, key material on these chips is
            // only as good as the bare RNG register, so say so out loud.
            warn!("No TRNG on this chip: RNG is not cryptographically secure until the radio is up");
            let rng = esp_hal::rng::Rng::new();
            register_custom_rng(rng);
        } else {
            // The RNG register only yields true randomness while an entropy
            // source is active. There are two, and this firmware uses both in
            // sequence:
            //
            //   1. the SAR ADC source enabled here, covering early boot, and
            //   2. the RF subsystem, once WiFi is up.
            //
            // `Trng::downgrade` returns `Rng`, a zero-sized handle that just
            // reads the register, so it carries no guarantee of its own —
            // whichever source is live at the time is what decides quality.
            //
            // The ADC source must therefore stay enabled until the radio takes
            // over, because everything minted in between depends on it: the
            // SSH host key, WiFi SSID/PSK and MAC all come from
            // `store::load_or_create` and `prepare_ap_config` below. It is
            // handed over (and explicitly dropped) just before the radio is
            // initialised — see the drop site further down for why it cannot
            // simply be left running.
            let trng_source = TrngSource::new(peripherals.RNG, peripherals.ADC1);
            let trng = Trng::try_new()
                .expect("TrngSource was just created, so the TRNG must be available");
            let rng = trng.downgrade();
            register_custom_rng(rng);
        }
    }

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

    // On first boot this mints the SSH host key and the WiFi PSK, so the
    // entropy source enabled above has to still be running. Guard the
    // invariant rather than trusting a comment: `debug-assertions` are on
    // even in release for this workspace, so reintroducing an early drop of
    // the `TrngSource` fails loudly on the bench instead of silently
    // producing predictable keys.
    #[cfg(not(any(feature = "esp32c5", feature = "esp32c61")))]
    debug_assert!(
        TrngSource::is_enabled(),
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
    .expect("Could not load or create SSHStampConfig");

    // Line settings for the bridge; the UART task is configured with them
    // below, so `SSH_STAMP_UART_*` changes take effect on the next boot.
    let uart_params = flash_config.uart_params;

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
        } else if #[cfg(feature = "esp32c6")] {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority10);
        } else {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority1);
        }
    }
    interrupt_spawner.spawn(
        uart_task(uart_buf, peripherals.UART1, pins, uart_params).expect("uart_task spawn failed"),
    );

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

    debug!("Initialising radio");

    // Last consumer of randomness before the radio: mints the WiFi PSK if the
    // config did not already carry one.
    let ap_config = app::prepare_ap_config(config, &platform)
        .await
        .expect("Failed to prepare AP config");

    // Hand the entropy source over to the radio.
    //
    // The SAR ADC source cannot simply be left running: Espressif requires it
    // to be switched off "before RF subsystem features, ADC, or I2S (ESP32
    // only) are initialized", warning that it "is not safe to use if any other
    // subsystem is accessing the RF subsystem or the ADC at the same time"
    // (ESP-IDF, Random Number Generation). It also commandeers the SAR ADC —
    // and I2S0 on the classic ESP32 — which the radio needs back.
    //
    // Nothing is lost by dropping it here: `WifiController::new` inside
    // `bring_up()` enables the RF subsystem, which is itself an entropy
    // source, and esp-radio registers that fact with esp-hal. So the SSH
    // session and key-exchange material sunset draws per connection is still
    // covered, just by the radio rather than the ADC.
    //
    // https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/random.html
    #[cfg(not(any(feature = "esp32c5", feature = "esp32c61")))]
    drop(trng_source);

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

    // The radio should have picked up the entropy duty dropped above:
    // esp-radio bumps esp-hal's entropy-source count once the RF subsystem is
    // running. sunset draws fresh key-exchange material from `getrandom` for
    // every SSH connection served below, so if this does not hold the handover
    // has a hole in it.
    #[cfg(not(any(feature = "esp32c5", feature = "esp32c61")))]
    debug_assert!(
        TrngSource::is_enabled(),
        "no entropy source active after WiFi came up"
    );

    if let Err(e) = app::run_app(stack.unwrap(), uart_buf, config, &platform).await {
        error!("run_app exited with error: {e}");
    }

    warn!("End of main, resetting");
    esp_hal::system::software_reset();
}

/// `getrandom` custom backend.
///
/// getrandom 0.4 picks its entropy backend by cfg rather than by cargo
/// feature: every bare-metal target here is built with
/// `--cfg getrandom_backend="custom"` (see `.cargo/config.toml`), which
/// makes getrandom link this symbol. It must be defined exactly once in the
/// whole program, so it lives in the binary — as getrandom's own docs
/// recommend — and forwards to the hardware TRNG registered at boot by
/// [`register_custom_rng`].
///
/// This is what feeds the SSH host key and the `WiFi` PSK, so it must be a
/// real entropy source, never a stub.
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    // SAFETY: getrandom guarantees `dest` is valid for writes of `len`
    // bytes. The buffer may be uninitialised, so it is zeroed before a
    // slice is formed over it, as getrandom's documentation prescribes.
    let buf = unsafe {
        core::ptr::write_bytes(dest, 0, len);
        core::slice::from_raw_parts_mut(dest, len)
    };
    ssh_stamp_esp32::rng_fill_bytes(buf)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
