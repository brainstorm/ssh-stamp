// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::future::Future;

/// Timer operations
pub trait TimerHal {
    /// Get current time in microseconds since boot
    fn now_micros(&self) -> u64;

    /// Get current time in milliseconds since boot
    fn now_millis(&self) -> u64 {
        self.now_micros() / 1000
    }

    /// Wait for specified duration in milliseconds
    fn delay(&self, millis: u64) -> impl Future<Output = ()>;
}
