// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The hardware tests for the entropy source.

#![no_std]
#![no_main]

#[embedded_test::tests(default_timeout = 30, executor = esp_rtos::embassy::Executor::new())]
mod tests {
    use esp_hal::peripherals::{ADC1, RNG, WIFI};
    use esp_radio::wifi::{ControllerConfig, WifiController};
    use ssh_stamp_esp32::entropy::{self, BootEntropy, NO_ENTROPY_SOURCE};

    pub struct Context {
        rng: RNG<'static>,
        adc: ADC1<'static>,
        wifi: WIFI<'static>,
    }

    #[init]
    fn init() -> Context {
        ssh_stamp_esp32::init_heap!();
        ssh_stamp_esp32::esp_bootloader_esp_idf::esp_app_desc!();
        ssh_stamp_esp32::logger::init();

        let peripherals = esp_hal::init(esp_hal::Config::default());
        let _sw_int1 = ssh_stamp_esp32::start_rtos!(peripherals);

        Context {
            rng: peripherals.RNG,
            adc: peripherals.ADC1,
            wifi: peripherals.WIFI,
        }
    }

    #[test]
    #[cfg_attr(
        any(feature = "esp32c5", feature = "esp32c61"),
        ignore = "no TRNG driver"
    )]
    fn random_needs_source(context: Context) {
        let mut buf = [0u8; 32];
        assert!(!entropy::is_active());
        assert_eq!(getrandom::fill(&mut buf), Err(NO_ENTROPY_SOURCE));

        let boot_entropy = BootEntropy::start(context.rng, context.adc);
        assert!(entropy::is_active());
        assert_eq!(getrandom::fill(&mut buf), Ok(()));

        drop(boot_entropy);
        assert!(!entropy::is_active());
        assert_eq!(getrandom::fill(&mut buf), Err(NO_ENTROPY_SOURCE));
    }

    #[test]
    fn fills_buffers_longer_than_block(context: Context) {
        let mut buf = [0u8; 150];
        let _boot_entropy = BootEntropy::start(context.rng, context.adc);
        assert_eq!(getrandom::fill(&mut buf), Ok(()));

        for part in buf.chunks(30) {
            assert!(part.iter().any(|&byte| byte != 0));
        }
    }

    #[test]
    #[cfg_attr(
        any(feature = "esp32c5", feature = "esp32c61"),
        ignore = "no TRNG driver"
    )]
    fn radio_takes_over(context: Context) {
        let mut buf = [0u8; 32];
        let boot_entropy = BootEntropy::start(context.rng, context.adc);

        let controller = boot_entropy
            .hand_over_to_radio(|| {
                assert_eq!(getrandom::fill(&mut buf), Err(NO_ENTROPY_SOURCE));
                WifiController::new(context.wifi, ControllerConfig::default())
            })
            .expect("starting the radio");
        assert!(entropy::is_active());
        assert_eq!(getrandom::fill(&mut buf), Ok(()));

        drop(controller);
        assert!(!entropy::is_active());
        assert_eq!(getrandom::fill(&mut buf), Err(NO_ENTROPY_SOURCE));
    }
}
