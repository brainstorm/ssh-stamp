// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The hardware device tests for the flash config storage.

#![no_std]
#![no_main]

// Links the HIL library crate for the bootloader and getrandom initialization.
use ssh_stamp_esp32_hil as _;

#[embedded_test::tests(default_timeout = 30, executor = esp_rtos::embassy::Executor::new())]
mod tests {
    use ssh_stamp::config::UartPins;
    use ssh_stamp::store;
    use ssh_stamp::store::{create, load};
    use ssh_stamp_esp32::{EntropySource, get_flash_n_buffer, boot};

    /// The test init state.
    pub struct Context {
        _entropy_source: EntropySource,
    }

    #[init]
    fn init() -> Context {
        boot!(_peripherals, _rng, entropy_source, _sw_int1);

        Context {
            _entropy_source: entropy_source,
        }
    }

    #[test]
    async fn config_flash_create_load(_context: Context) {
        let flash_guard = get_flash_n_buffer().expect("flash was initialised in init");
        let mut fb = flash_guard.lock().await;
        let (flash, buf) = fb.split_ref_mut();

        // Define a test mac to avoid confusion with any real board.
        let mac = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        let pins = UartPins { rx: 10, tx: 11 };

        let created =
            create(flash, buf, mac, pins).expect("saving a config");
        let loaded = load(flash, buf).expect("reading the config back");

        assert_eq!(loaded.mac, mac);
        assert_eq!(loaded.uart_pins, UartPins { rx: 10, tx: 11 });
        assert_eq!(loaded.wifi_ap_ssid, created.wifi_ap_ssid);
        // Avoid using assert_eq to not show any PSK or secret values on failure.
        #[allow(clippy::manual_assert_eq)]
        assert!(created == loaded);
    }
}
