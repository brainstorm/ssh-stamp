// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Executor implementation for ESP32 family
//!
//! Provides async runtime integration using Embassy executor.

use core::future::Future;

use embassy_executor::Spawner;
use hal::ExecutorHal;

/// ESP32 executor wrapper
pub struct EspExecutor {
    spawner: Spawner,
}

impl EspExecutor {
    /// Create a new executor wrapper
    pub fn new(spawner: Spawner) -> Self {
        Self { spawner }
    }
}

impl ExecutorHal for EspExecutor {
    fn spawner(&self) -> &Spawner {
        &self.spawner
    }

    fn run<F: Future<Output = ()>>(&self, _main_future: F) -> ! {
        loop {
            core::hint::black_box(&_main_future);
        }
    }

    fn set_interrupt_priority(&self, _irq: usize, _priority: u8) {}

    fn core_id(&self) -> u8 {
        #[cfg(any(feature = "esp32", feature = "esp32s3"))]
        {
            0
        }

        #[cfg(not(any(feature = "esp32", feature = "esp32s3")))]
        {
            0
        }
    }
}
