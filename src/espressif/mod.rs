// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! ESP32 platform support
//!
//! This module provides app-specific wrappers around the HAL implementations.

pub mod buffered_uart;
pub mod net;

// Re-export RNG registration from hal-espressif
pub use hal_espressif::EspRng;

/// Register the hardware RNG for use with getrandom
pub fn register_custom_rng(rng: esp_hal::rng::Rng) {
    hal_espressif::EspRng::register(rng);
}
