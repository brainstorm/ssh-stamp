// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Time.
//!
//! `bl616-wifi` registers the embassy time driver over the `FreeRTOS` tick, so
//! this is the same thin wrapper over `embassy_time` the ESP port uses. The
//! tick is 1 kHz, so `now_micros` has millisecond granularity despite its
//! name -- callers that need finer resolution than that need a different
//! clock, not a different wrapper.

use ssh_stamp_hal::TimerHal;

/// The HAL's clock.
pub struct Bl616Timer;

impl TimerHal for Bl616Timer {
    fn now_micros(&self) -> u64 {
        embassy_time::Instant::now().as_micros()
    }

    async fn delay(&self, millis: u64) {
        embassy_time::Timer::after_millis(millis).await;
    }
}
