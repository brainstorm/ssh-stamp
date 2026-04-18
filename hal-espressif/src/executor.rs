// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Executor integration for ESP32 family.
//!
//! The actual executor lifecycle is managed by `esp_rtos`/embassy directly in main.rs.
//! No HAL abstraction is needed since embassy is used across all MCU targets.

use embassy_executor::Spawner;

/// ESP32 executor wrapper
pub struct EspExecutor {
    spawner: Spawner,
}

impl EspExecutor {
    #[must_use]
    pub fn new(spawner: Spawner) -> Self {
        Self { spawner }
    }

    #[must_use]
    pub fn spawner(&self) -> &Spawner {
        &self.spawner
    }
}
