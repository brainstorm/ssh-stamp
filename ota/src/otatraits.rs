// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

// use embedded_storage::Storage;
// use esp_bootloader_esp_idf;
// use esp_hal::system;
// use storage::flash;

#[allow(unused_imports)]
use log::{debug, error, info, warn};

#[derive(Debug)]
pub enum StorageError {
    ReadError,
    WriteError,
    EraseError,
    InternalError,
}

pub type StorageResult<T> = Result<T, StorageError>;

/// Any target hardware vendor supporting ota for ssh-stamp requires an implementation of this trait.
/// This facilitates the integration of the OTA across platforms
pub trait OtaActions {
    /// This function tries to validate the current loaded OTA partition.
    ///
    /// A failure in this validation might means that the OTA will be rolled back in the next reboot
    fn try_validating_current_ota_partition()
    -> impl core::future::Future<Output = StorageResult<()>> + Send;

    /// Obtains the space available for writing OTAs
    fn get_ota_partition_size() -> impl core::future::Future<Output = StorageResult<u32>> + Send;

    /// Writes data in the given offset as part of an OTA transfer process
    fn write_ota_data(
        &self,
        offset: u32,
        data: &[u8],
    ) -> impl core::future::Future<Output = StorageResult<()>> + Send;

    /// Completes the ota process
    ///
    /// Final checks if the platworm requires it
    fn finalize_ota_update(
        &mut self,
    ) -> impl core::future::Future<Output = StorageResult<()>> + Send;

    /// Resets the target device to apply the OTA update
    ///
    /// This function should not return
    fn reset_device(&self) -> !;
}
