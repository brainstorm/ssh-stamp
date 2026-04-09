// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Flash storage and OTA update traits.

use core::future::Future;

use crate::HalError;

/// Flash storage operations.
///
/// Provides read/write/erase operations for non-volatile flash memory.
/// Flash must be erased before writing; see [`Self::erase`] for details.
///
/// # Example
///
/// ```ignore
/// async fn write_config<F: FlashHal>(flash: &F, config: &[u8]) -> Result<(), HalError> {
///     let offset = 0x1000;
///     flash.erase(offset, config.len() as u32).await?;
///     flash.write(offset, config).await?;
///     Ok(())
/// }
/// ```
pub trait FlashHal {
    /// Read from flash at offset.
    ///
    /// Reads `buf.len()` bytes starting at `offset` into the buffer.
    ///
    /// # Arguments
    ///
    /// * `offset` - Byte offset within flash to read from.
    /// * `buf` - Destination buffer for read data.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or a [`HalError::Flash`] error on failure.
    fn read(&self, offset: u32, buf: &mut [u8]) -> impl Future<Output = Result<(), HalError>>;

    /// Write to flash at offset.
    ///
    /// Writes data to flash. The target region must be erased first
    /// (flash cannot transition from 0 to 1 bits without erasing).
    ///
    /// # Arguments
    ///
    /// * `offset` - Byte offset within flash to write to.
    /// * `buf` - Data to write.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or a [`HalError::Flash`] error on failure.
    fn write(&self, offset: u32, buf: &[u8]) -> impl Future<Output = Result<(), HalError>>;

    /// Erase flash region.
    ///
    /// Sets all bits in the region to 1 (ready for writing).
    /// Flash must be erased before writing; writes can only change 1s to 0s.
    ///
    /// # Arguments
    ///
    /// * `offset` - Byte offset within flash to start erasing.
    /// * `len` - Number of bytes to erase (will be rounded up to erase block size).
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or a [`HalError::Flash`] error on failure.
    fn erase(&self, offset: u32, len: u32) -> impl Future<Output = Result<(), HalError>>;

    /// Get flash storage size in bytes.
    ///
    /// Returns the total size of the flash storage region available
    /// for application use.
    fn size(&self) -> u32;
}

/// OTA update operations.
///
/// Implementations handle platform-specific partition management
/// for Over-The-Air firmware updates.
///
/// # Safety
///
/// OTA updates can brick the device if interrupted or corrupted.
/// Implementations should validate firmware before marking as bootable.
///
/// # Example
///
/// ```ignore
/// async fn perform_ota<O: OtaActions>(ota: &mut O, firmware: &[u8]) -> Result<(), HalError> {
///     O::try_validating_current_ota_partition().await?;
///     let size = O::get_ota_partition_size().await?;
///     
///     for (offset, chunk) in firmware.chunks(4096).enumerate() {
///         ota.write_ota_data((offset * 4096) as u32, chunk).await?;
///     }
///     
///     ota.finalize_ota_update().await?;
///     ota.reset_device();
/// }
/// ```
pub trait OtaActions {
    /// Validate the current OTA partition.
    ///
    /// Marks the current OTA slot as VALID. This is only needed if the bootloader
    /// was built with auto-rollback support to prevent reverting to a failed update.
    ///
    /// # Returns
    ///
    /// `Ok(())` on successful validation, or an error if validation fails.
    fn try_validating_current_ota_partition() -> impl Future<Output = Result<(), HalError>> + Send;

    /// Get size of OTA partition in bytes.
    ///
    /// Returns the total size available for new firmware in the update partition.
    ///
    /// # Returns
    ///
    /// Partition size in bytes on success, or an error if partition not found.
    fn get_ota_partition_size() -> impl Future<Output = Result<u32, HalError>> + Send;

    /// Write data to OTA partition at offset.
    ///
    /// Writes a chunk of firmware data to the update partition.
    /// Call this repeatedly for each chunk of the firmware image.
    ///
    /// # Arguments
    ///
    /// * `offset` - Byte offset within the partition.
    /// * `data` - Firmware data chunk to write.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or an error on write failure.
    fn write_ota_data(
        &self,
        offset: u32,
        data: &[u8],
    ) -> impl Future<Output = Result<(), HalError>> + Send;

    /// Finalize OTA update and mark for boot.
    ///
    /// Validates the written firmware and marks the partition as bootable.
    /// After calling this, [`Self::reset_device`] should be called to boot into the new firmware.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or an error if validation or marking fails.
    fn finalize_ota_update(&mut self) -> impl Future<Output = Result<(), HalError>> + Send;

    /// Reset device to boot into new partition.
    ///
    /// Triggers a system reset. The bootloader will boot from the newly marked
    /// OTA partition. This function never returns.
    fn reset_device(&self) -> !;
}
