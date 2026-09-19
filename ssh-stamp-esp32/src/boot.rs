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

//! The boot sequence macros which are used when initializing the device. These
//! are separate macros so that they remain testable. Everything that consumes
//! `Peripherals` must be a macro because fields like `TIMG1` or `SYSTIMER` are
//! different per-chip, and cannot be resolved in a single function in the
//! library crate.

use embassy_executor::SendSpawner;
use esp_hal::interrupt::{Priority, software::SoftwareInterrupt};
use esp_rtos::embassy::InterruptExecutor;
use static_cell::StaticCell;

/// Creates the global heap allocator using [`ssh_stamp::settings::HEAP_SIZE`].
///
/// The calling crate needs `ssh-stamp` and `esp-hal` as dependencies.
#[macro_export]
macro_rules! init_heap {
    () => {
        // TODO: This heap size will crash at runtime (only for the ESP32S2);
        // see https://github.com/brainstorm/ssh-stamp/pull/41#issuecomment-2964775170
        #[cfg(feature = "esp32s2")]
        $crate::esp_alloc::heap_allocator!(#[$crate::esp_hal::ram(reclaimed)] size: $crate::ssh_stamp::settings::HEAP_SIZE);
        #[cfg(not(feature = "esp32s2"))]
        $crate::esp_alloc::heap_allocator!(size: $crate::ssh_stamp::settings::HEAP_SIZE);
    };
}

/// Starts the esp-rtos scheduler on `TIMG1` for original ESP32 and `SYSTIMER` everywhere
/// else. This  registers the embassy time driver, so it must run before anything that
/// uses `embassy-time`.
///
/// The scheduler uses the software interrupt 0 for context switching, so the
/// macro consumes `SW_INTERRUPT`: [`SoftwareInterrupt<1>`](SoftwareInterrupt).
#[macro_export]
macro_rules! start_rtos {
    ($peripherals:ident) => {{
        let sw_int = $crate::esp_hal::interrupt::software::SoftwareInterruptControl::new(
            $peripherals.SW_INTERRUPT,
        );
        #[cfg(feature = "esp32")]
        $crate::esp_rtos::start(
            $crate::esp_hal::timer::timg::TimerGroup::new($peripherals.TIMG1).timer0,
            sw_int.software_interrupt0,
        );
        #[cfg(not(feature = "esp32"))]
        $crate::esp_rtos::start(
            $crate::esp_hal::timer::systimer::SystemTimer::new($peripherals.SYSTIMER).alarm0,
            sw_int.software_interrupt0,
        );
        sw_int.software_interrupt1
    }};
}

/// The ssh-stamp boot sequence, which does heap allocation, `esp_hal::init`,
/// configures the entropy source, flash storage and the esp-rtos scheduler.
///
/// The `Peripherals` struct cannot be returned once fields have been moved
/// out of it, so the caller names the bindings and the macro introduces
/// them into scope, e.g:
///
/// ```ignore
/// ssh_stamp_esp32::boot!(peripherals, rng, entropy_source, sw_int1);
/// ```
#[macro_export]
macro_rules! boot {
    ($peripherals:ident, $rng:ident, $entropy_source:ident, $sw_int1:ident) => {
        $crate::init_heap!();
        $crate::esp_bootloader_esp_idf::esp_app_desc!();
        $crate::esp_println::logger::init_logger_from_env();
        $crate::bench::log_heap("boot");
        $crate::log::debug!("HSM: initialising peripherals");

        // Note that benches do depend on a stable clock speed across comparisons. The default
        // shouldn't change much, but theoretically an upgrade could change it.
        //
        // `mut` is only exercised by `setup_can_transceiver!`'s reborrows on
        // boards whose CAN mux shares the I2C bus with the `i2c` subsystem.
        #[allow(unused_mut)]
        let mut $peripherals = $crate::esp_hal::init($crate::esp_hal::Config::default());

        // Enable true random number generation before the config is created, so
        // the WiFi password and SSH host key have cryptographically secure values.
        let ($rng, $entropy_source) = $crate::init_entropy!($peripherals);

        $crate::flash_init($peripherals.FLASH);
        let $sw_int1 = $crate::start_rtos!($peripherals);
    };
}

/// Starts the `InterruptExecutor` on the [`SoftwareInterrupt<1>`](SoftwareInterrupt) left over
/// from  [`start_rtos!`](macro@crate::start_rtos), and returns its spawner.
pub fn start_interrupt_executor(sw_int1: SoftwareInterrupt<'static, 1>) -> SendSpawner {
    static INT_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new(); // 0 is used for esp_rtos

    let interrupt_executor = INT_EXECUTOR.init_with(|| InterruptExecutor::new(sw_int1));
    cfg_if::cfg_if! {
        if #[cfg(any(feature = "esp32", feature = "esp32s2", feature = "esp32s3"))] {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority3);
        } else if #[cfg(feature = "esp32c6")] {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority10);
        } else {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority1);
        }
    }
    interrupt_spawner
}
