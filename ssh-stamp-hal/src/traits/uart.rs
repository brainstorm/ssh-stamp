// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 gabriel.ku <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! UART hardware abstraction trait.

use core::future::Future;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

use crate::{HalError, UartConfig};

/// UART hardware abstraction.
///
/// Provides asynchronous read/write operations for UART peripherals.
/// Implementations should handle buffering and interrupt management internally.
///
/// # Example
///
/// ```ignore
/// async fn echo<U: UartHal>(uart: &mut U) -> Result<(), HalError> {
///     let mut buf = [0u8; 64];
///     let n = uart.read(&mut buf).await?;
///     uart.write(&buf[..n]).await?;
///     Ok(())
/// }
/// ```
pub trait UartHal {
    /// Read bytes into buffer.
    ///
    /// Fills the buffer with received data and returns the number of bytes read.
    /// This method waits until at least one byte is available.
    ///
    /// # Arguments
    ///
    /// * `buf` - Destination buffer for received data.
    ///
    /// # Returns
    ///
    /// Number of bytes read on success, or an error on failure.
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize, HalError>>;

    /// Write bytes from buffer.
    ///
    /// Transmits all bytes from the buffer. This method returns once all
    /// bytes have been queued for transmission (may still be in hardware buffers).
    ///
    /// # Arguments
    ///
    /// * `buf` - Data to transmit.
    ///
    /// # Returns
    ///
    /// Number of bytes written on success (always equal to `buf.len()`), or an error.
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = Result<usize, HalError>>;

    /// Check if data is available to read.
    ///
    /// Returns `true` if at least one byte is available in the receive buffer.
    /// This is a non-blocking check useful for polling patterns.
    fn can_read(&self) -> bool;

    /// Get async notification signal.
    ///
    /// Returns a signal that is signalled when data becomes available for reading.
    /// This enables efficient async/await patterns where the caller can wait
    /// for the signal instead of polling [`Self::can_read`].
    fn signal(&self) -> &Signal<CriticalSectionRawMutex, ()>;

    /// Reconfigure UART with new settings.
    ///
    /// Changes the UART configuration (baud rate, pins, etc.) at runtime.
    /// This may temporarily disrupt ongoing transfers.
    ///
    /// # Arguments
    ///
    /// * `config` - New UART configuration to apply.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or an error if configuration fails.
    fn reconfigure(&mut self, config: UartConfig) -> impl Future<Output = Result<(), HalError>>;
}
