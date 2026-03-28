// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::future::Future;

use crate::{HalError, UartConfig};

/// UART hardware abstraction
pub trait UartHal {
    /// Read bytes into buffer, returns number of bytes read
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize, HalError>>;

    /// Write bytes from buffer, returns number of bytes written
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = Result<usize, HalError>>;

    /// Check if data is available to read
    fn can_read(&self) -> bool;

    /// Signal for async notification when data is available
    fn signal(
        &self,
    ) -> &embassy_sync::signal::Signal<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, ()>;

    /// Reconfigure UART with new settings
    fn reconfigure(&mut self, config: UartConfig) -> impl Future<Output = Result<(), HalError>>;
}
