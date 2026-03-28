// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::future::Future;

use crate::HalError;

/// Flash storage operations
pub trait FlashHal {
    /// Read from flash at offset
    fn read(&self, offset: u32, buf: &mut [u8]) -> impl Future<Output = Result<(), HalError>>;

    /// Write to flash at offset (must be erased first)
    fn write(&self, offset: u32, buf: &[u8]) -> impl Future<Output = Result<(), HalError>>;

    /// Erase flash region
    fn erase(&self, offset: u32, len: u32) -> impl Future<Output = Result<(), HalError>>;

    /// Get flash storage size in bytes
    fn size(&self) -> u32;
}

/// OTA update operations
///
/// Implementations handle platform-specific partition management
/// for Over-The-Air firmware updates.
pub trait OtaActions {
    /// Validate the current OTA partition
    ///
    /// Mark the current OTA slot as VALID - this is only needed if the bootloader
    /// was built with auto-rollback support.
    fn try_validating_current_ota_partition() -> impl Future<Output = Result<(), HalError>> + Send;

    /// Get size of OTA partition in bytes
    fn get_ota_partition_size() -> impl Future<Output = Result<u32, HalError>> + Send;

    /// Write data to OTA partition at offset
    fn write_ota_data(
        &self,
        offset: u32,
        data: &[u8],
    ) -> impl Future<Output = Result<(), HalError>> + Send;

    /// Finalize OTA update and mark for boot
    fn finalize_ota_update(&mut self) -> impl Future<Output = Result<(), HalError>> + Send;

    /// Reset device to boot into new partition
    fn reset_device(&self) -> !;
}
