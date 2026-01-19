// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::handler::OtaError;

use embedded_storage::Storage;
use esp_bootloader_esp_idf;
use esp_hal::system;
use storage::flash;

use log::{debug, error, info, warn};

#[derive(Debug)]
pub enum FlashError {
    ReadError,
    WriteError,
    EraseError,
    InternalError,
}

pub type FlashResult<T> = Result<T, FlashError>;

/// This structure is meant to wrap the media access for writing the OTA firmware
/// It will abstract the flash memory or other storage media so later we can implement it for different platforms
#[derive(Debug, Copy, Clone)]
pub(crate) struct OtaWriter {}

impl OtaWriter {
    /// Creates a new OtaWriter for the given target OTA slot.
    ///
    /// To obtain a target OTA slot use [get_next_app_slot]
    pub(crate) fn new() -> Self {
        OtaWriter {}
    }
    // TODO: Not tested. May crash!
    /// Writes data to the target OTA partition at the given offset.
    pub(crate) async fn write(&self, offset: u32, data: &[u8]) -> FlashResult<()> {
        write_to_target(offset, data).await
    }
    // TODO: Not tested. May crash!
    /// Finalizes the OTA update by setting the target slot as current.
    pub async fn finalize(&mut self) -> FlashResult<()> {
        activate_next_ota_slot().await?;
        system::software_reset(); // TODO: Not the right place. I would need to signal the main app to reboot after closing the SFTP session
    }
}

/// Finds the next app slot to write the OTA update to.
///
/// We use an slot since it does not require lifetimes and is easier to handle.
// Tested with espflash md5 and espflash write-bin. Writing with SFTP seems to work fine.
async fn write_to_target(offset: u32, data: &[u8]) -> FlashResult<()> {
    let mut fb = flash::get_flash_n_buffer().lock().await;
    let (storage, buffer) = fb.split_ref_mut();
    let Ok(pt) = esp_bootloader_esp_idf::partitions::read_partition_table(
        storage,
        &mut buffer[..esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN],
    ) else {
        error!("Could not read partition table");
        return Err(FlashError::ReadError);
    };

    // Next section requires a version bump to esp_storage to 0.8.1
    // let mut ota =
    //     esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_part_table)
    //         .map_err(|e| {
    //             error!("Could not create OtaUpdater: {:?}", e);
    //             FlashError::InternalError
    //         })?;
    // let (mut target_partition, part_type) = ota.next_partition().map_err(|e| {
    //     error!("Could not get next partition: {:?}", e);
    //     FlashError::InternalError
    // })?;

    // debug!("Flashing image to {:?}", part_type);

    // debug!(
    //     "Writing data to target_partition at offset {}, with len {}",
    //     offset,
    //     data.len()
    // );
    // target_partition.write(offset, data).map_err(|e| {
    //     error!("Failed to write data to target_partition: {}", e);
    //     FlashError::WriteError
    // })?;

    Ok(())
}

// TODO: Does not crash but the OTADATA partition is invalid on boot
/// The provided target slot will be marked as current and the image will be set as New so after
/// the reboot it will boot from it and be validated if the bootloader requires it.
///
/// We use a slot since it does not require lifetimes and is easier to handle.
async fn activate_next_ota_slot() -> FlashResult<()> {
    let mut fb = flash::get_flash_n_buffer().lock().await;
    let (storage, buffer) = fb.split_ref_mut();

    let (mut buff_part_table, _) = buffer
        .split_at_mut_checked(esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN)
        .ok_or({
            error!("Could not split buffer for partition table");
            FlashError::InternalError
        })?;
    let pt =
        esp_bootloader_esp_idf::partitions::read_partition_table(storage, &mut buff_part_table)
            .map_err(|e| {
                error!("Could not read partition table: {:?}", e);
                FlashError::ReadError
            })?;

    debug!("Currently booted partition {:?}", pt.booted_partition());

    // Next section requires a version bump to esp_storage to 0.8.1
    // let mut ota =
    //     esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_part_table)
    //         .map_err(|e| {
    //             error!("Could not create OtaUpdater: {:?}", e);
    //             FlashError::InternalError
    //         })?;
    // ota.activate_next_partition().map_err(|e| {
    //     error!("Could not activate next partition: {:?}", e);
    //     FlashError::WriteError
    // })?;
    // ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::New)
    //     .map_err(|e| {
    //         error!("Could not set OTA image state to New: {:?}", e);
    //         FlashError::WriteError
    //     })?;

    Ok(())
}

// TODO: build bootloader with auto-rollback to avoid invalid images rendering the device unbootable
// TODO: Report bug in OtaImageState to esp
/// Validate the current OTA partition
///
/// Mark the current OTA slot as VALID - this is only needed if the bootloader was built with auto-rollback support.
/// The default pre-compiled bootloader in espflash is NOT.
///
pub async fn try_validating_current_ota_partition() -> FlashResult<()> {
    /// Taken from [esp-rs ota_update example](https://github.com/esp-rs/esp-hal/blob/99042a7d60388580459eab6fe0d10e2f89d6ab6c/examples/src/bin/ota_update.rs)
    let mut fb = flash::get_flash_n_buffer().lock().await;
    let (storage, buffer) = fb.split_ref_mut();

    let (mut buff_part_table, _) = buffer
        .split_at_mut_checked(esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN)
        .ok_or({
            error!("Could not split buffer for partition table");
            FlashError::InternalError
        })?;
    let pt =
        esp_bootloader_esp_idf::partitions::read_partition_table(storage, &mut buff_part_table)
            .map_err(|e| {
                error!("Could not read partition table: {:?}", e);
                FlashError::InternalError
            })?;

    debug!("Currently booted partition {:?}", pt.booted_partition());

    // Next section requires a version bump to esp_storage
    // let mut ota =
    //     esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_part_table)
    //         .map_err(|e| {
    //             error!("Could not create OtaUpdater: {:?}", e);
    //             FlashError::InternalError
    //         })?;
    // let current = ota.selected_partition().map_err(|e| {
    //     error!("Could not get selected partition: {:?}", e);
    //     FlashError::InternalError
    // })?;

    // debug!(
    //     "current image state {:?} (only relevant if the bootloader was built with auto-rollback support)",
    //     ota.current_ota_state()
    // );
    // debug!("currently selected partition {:?}", current);

    // if let Ok(state) = ota.current_ota_state() {
    //     if state == esp_bootloader_esp_idf::ota::OtaImageState::New
    //         || state == esp_bootloader_esp_idf::ota::OtaImageState::PendingVerify
    //     {
    //         ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::Valid)
    //             .map_err(|e| {
    //                 error!("Could not set OTA image state to Valid: {:?}", e);
    //                 FlashError::WriteError
    //             })?;
    //         debug!("Changed state to VALID");
    //     }
    // }

    Ok(())
}
