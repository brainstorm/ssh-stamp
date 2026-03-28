// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Timer implementation for ESP32 family
//!
//! Provides microsecond and millisecond timing using ESP32 hardware timers.

use embassy_time::{Duration, Instant};
use hal::TimerHal;

/// ESP32 Timer implementation using Embassy time
pub struct EspTimer;

impl TimerHal for EspTimer {
    fn now_micros(&self) -> u64 {
        Instant::now().as_micros()
    }

    async fn delay(&self, millis: u64) {
        embassy_time::Timer::after(Duration::from_millis(millis)).await;
    }
}
