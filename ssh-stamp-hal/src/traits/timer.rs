// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Timer operations trait.

use core::future::Future;

/// Timer hardware abstraction.
///
/// Provides time measurement and delays. Implementations typically wrap
/// system tick timers or RTOS timer facilities.
///
/// # Example
///
/// ```ignore
/// async fn measure_time<T: TimerHal>(timer: &T) -> u64 {
///     let start = timer.now_millis();
///     some_operation().await;
///     timer.now_millis() - start
/// }
/// ```
pub trait TimerHal {
    /// Get current time in microseconds since boot.
    ///
    /// Returns a monotonically increasing counter of microseconds since
    /// system startup. May wrap around on long-running systems.
    fn now_micros(&self) -> u64;

    /// Get current time in milliseconds since boot.
    ///
    /// Convenience wrapper around [`Self::now_micros`] with millisecond resolution.
    fn now_millis(&self) -> u64 {
        self.now_micros() / 1000
    }

    /// Wait for specified duration.
    ///
    /// Asynchronously waits for the specified number of milliseconds.
    /// This is an async operation that yields to the executor.
    ///
    /// # Arguments
    ///
    /// * `millis` - Duration to wait in milliseconds.
    fn delay(&self, millis: u64) -> impl Future<Output = ()>;
}
