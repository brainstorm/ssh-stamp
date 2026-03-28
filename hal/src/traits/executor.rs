// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::future::Future;

use embassy_executor::Spawner;

/// Async runtime/executor operations
///
/// Provides access to the async runtime and interrupt management.
/// Implementations wrap platform-specific executors (e.g., esp-rtos for ESP32).
pub trait ExecutorHal {
    /// Get the spawner for spawning async tasks
    fn spawner(&self) -> &Spawner;

    /// Run the executor with the main future (blocking)
    fn run<F: Future<Output = ()>>(&self, main_future: F) -> !;

    /// Set interrupt priority
    fn set_interrupt_priority(&self, irq: usize, priority: u8);

    /// Get current core ID (for multi-core systems)
    fn core_id(&self) -> u8;
}
