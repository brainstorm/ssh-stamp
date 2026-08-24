// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Top-level library module.

use std::thread::sleep;
use std::time::{Duration, Instant};

/// A helper function to call the `operation` until it succeeds or the `timeout` passes, sleeping
/// for the `interval` between attempts.
pub fn retry<T, E>(
    timeout: Duration,
    interval: Duration,
    mut operation: impl FnMut() -> Result<T, E>,
) -> Result<T, E> {
    let deadline = Instant::now() + timeout;
    loop {
        match operation() {
            Err(_) if Instant::now() < deadline => sleep(interval),
            result => return result,
        }
    }
}