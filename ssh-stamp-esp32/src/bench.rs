// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! ESP32 heap instrumentation for benchmarking.
//!
//! This is the Platform-specific analogue to [`ssh_stamp::mem_probe`], as it requires
//! `esp_alloc::HEAP` to emit logs.
//!

use esp_alloc::HEAP;
use ssh_stamp::bench_emit;

/// Logs the global heap's usage using the label.
#[cfg(feature = "mem-probe")]
pub fn log_heap(label: &str) {
    let stats = HEAP.stats();
    bench_emit!(
        "heap={label} used_bytes={} total_bytes={} max_bytes={} alloc_bytes={} freed_bytes={}",
        stats.current_usage,
        stats.size,
        stats.max_usage,
        stats.total_allocated,
        stats.total_freed,
    );
}

/// Empty function, compiles to a no-op if `mem-probe` is not enabled.
#[cfg(not(feature = "mem-probe"))]
pub fn log_heap(_label: &str) {}
