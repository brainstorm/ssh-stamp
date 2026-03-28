// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Async runtime/executor operations trait.

use core::future::Future;

use embassy_executor::Spawner;

/// Async executor hardware abstraction.
///
/// Provides access to the async runtime and interrupt management.
/// Implementations wrap platform-specific executors (e.g., embassy-executor for ESP32).
///
/// # Example
///
/// ```ignore
/// use embassy_executor::Spawner;
///
/// async fn run_app<E: ExecutorHal>(executor: &E) {
///     let spawner = executor.spawner();
///     spawner.spawn(my_task()).ok();
///     executor.run(async {
///         // main application loop
///     });
/// }
/// ```
pub trait ExecutorHal {
    /// Get the spawner for spawning async tasks.
    ///
    /// Returns a reference to the executor's spawner, which can be used
    /// to spawn additional async tasks.
    fn spawner(&self) -> &Spawner;

    /// Run the executor with the main future.
    ///
    /// Blocks forever, running the executor and processing the main future.
    /// This method never returns.
    ///
    /// # Arguments
    ///
    /// * `main_future` - The main async task to run (typically the application entry point).
    fn run<F: Future<Output = ()>>(&self, main_future: F) -> !;

    /// Set interrupt priority.
    ///
    /// Configures the priority level for a specific interrupt. Lower priority
    /// numbers typically mean higher priority (platform-specific).
    ///
    /// # Arguments
    ///
    /// * `irq` - Interrupt number/identifier.
    /// * `priority` - Priority level (platform-specific interpretation).
    fn set_interrupt_priority(&self, irq: usize, priority: u8);

    /// Get current core ID.
    ///
    /// Returns the ID of the currently executing core. Useful for
    /// multi-core systems where different cores may need different handling.
    fn core_id(&self) -> u8;
}
