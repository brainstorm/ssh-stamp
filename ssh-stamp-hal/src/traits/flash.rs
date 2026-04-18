// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! OTA update traits.
//!
//! Flash storage operations should use `embedded_storage_async::nor_flash::NorFlash`
//! from the embedded-hal ecosystem rather than a custom trait.
//!
//! OTA update actions are kept here because they are application-specific
//! (partition management, firmware validation) and not covered by embedded-hal.

use core::future::Future;

use crate::HalError;

/// OTA update operations.
///
/// # Errors
///
/// All methods return `HalError` on failure.
pub trait OtaActions {
    /// Validate the current OTA partition.
    fn try_validating_current_ota_partition() -> impl Future<Output = Result<(), HalError>> + Send;

    /// Get size of OTA partition in bytes.
    fn get_ota_partition_size() -> impl Future<Output = Result<u32, HalError>> + Send;

    /// Write data to OTA partition at offset.
    fn write_ota_data(
        &self,
        offset: u32,
        data: &[u8],
    ) -> impl Future<Output = Result<(), HalError>> + Send;

    /// Finalize OTA update and mark for boot.
    fn finalize_ota_update(&mut self) -> impl Future<Output = Result<(), HalError>> + Send;

    /// Reset device to boot into new partition.
    fn reset_device(&self) -> !;
}
