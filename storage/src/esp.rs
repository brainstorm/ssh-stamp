#[allow(unused_imports)]
use log::{debug, error, info, warn};

use crate::flash;
use embedded_storage::Storage;
use esp_bootloader_esp_idf;
use esp_hal::system;
use ota::storagetraits::{OtaActions, StorageError, StorageResult};

/// This structure is meant to wrap the media access for writing the OTA firmware
/// It will abstract the flash memory or other storage media so later we can implement it for different platforms
#[derive(Debug, Copy, Clone)]
pub struct OtaWriter {}

impl OtaWriter {
    /// Creates a new OtaWriter for the given target OTA slot.
    ///
    /// To obtain a target OTA slot use [get_next_app_slot]
    pub fn new() -> Self {
        OtaWriter {}
    }
}

impl OtaActions for OtaWriter {
    // TODO: build bootloader with auto-rollback to avoid invalid images rendering the device unbootable
    // TODO: Report bug in OtaImageState to esp
    /// Validate the current OTA partition
    ///
    /// Mark the current OTA slot as VALID - this is only needed if the bootloader was built with auto-rollback support.
    /// The default pre-compiled bootloader in espflash is NOT.
    ///
    async fn try_validating_current_ota_partition() -> StorageResult<()> {
        // Taken from [esp-hal ota_update example](https://github.com/esp-rs/esp-hal/examples/src/bin/ota_update.rs)
        let Some(fb) = flash::get_flash_n_buffer() else {
            error!("Flash storage not initialized");
            return Err(StorageError::InternalError);
        };
        let mut fb = fb.lock().await;

        let (storage, _buffer) = fb.split_ref_mut();

        // OtaUpdater is very particular. It needs a mut ref of a buffer of the exact size of the partition table.
        // This is why we create it here and did not reuse the buffer from fb.
        let mut buff_ota = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];

        let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_ota)
            .map_err(|e| {
                error!("Could not create OtaUpdater: {:?}", e);
                StorageError::InternalError
            })?;
        let current = ota.selected_partition().map_err(|e| {
            error!("Could not get selected partition: {:?}", e);
            StorageError::InternalError
        })?;

        debug!(
            "current image state {:?} (only relevant if the bootloader was built with auto-rollback support)",
            ota.current_ota_state()
        );
        debug!("currently selected partition {:?}", current);

        if let Ok(state) = ota.current_ota_state() {
            if state == esp_bootloader_esp_idf::ota::OtaImageState::New
                || state == esp_bootloader_esp_idf::ota::OtaImageState::PendingVerify
            {
                ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::Valid)
                    .map_err(|e| {
                        error!("Could not set OTA image state to Valid: {:?}", e);
                        StorageError::WriteError
                    })?;
                debug!("Changed state to VALID");
            }
        }

        Ok(())
    }

    /// Gets the size of the target OTA partition.
    async fn get_ota_partition_size() -> StorageResult<u32> {
        let partition_size =
            u32::try_from(next_ota_size().await?).map_err(|_| StorageError::InternalError)?;
        Ok(partition_size)
    }
    // TODO: Not tested. May crash!
    /// Writes data to the target OTA partition at the given offset.
    async fn write_ota_data(&self, offset: u32, data: &[u8]) -> StorageResult<()> {
        write_to_target(offset, data).await
    }
    // TODO: Not tested. May crash!
    /// Finalizes the OTA update by setting the target slot as current.
    async fn finalize_ota_update(&mut self) -> StorageResult<()> {
        activate_next_ota_slot().await?;
        system::software_reset(); // TODO: Not the right place. I would need to signal the main app to reboot after closing the SFTP session
    }
}

/// Gets the size of the next OTA partition.
async fn next_ota_size() -> StorageResult<usize> {
    let Some(fb) = flash::get_flash_n_buffer() else {
        error!("Flash storage not initialized");
        return Err(StorageError::InternalError);
    };
    let mut fb = fb.lock().await;

    let (storage, _buffer) = fb.split_ref_mut();

    // OtaUpdater is very particular. It needs a mut ref of a buffer of the exact size of the partition table.
    // This is why we create it here and did not reuse the buffer from fb.
    let mut buff_ota = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];

    let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_ota)
        .map_err(|e| {
            error!("Could not create OtaUpdater: {:?}", e);
            StorageError::InternalError
        })?;
    let (target_partition, _) = ota.next_partition().map_err(|e| {
        error!("Could not get next partition: {:?}", e);
        StorageError::InternalError
    })?;

    Ok(target_partition.partition_size())
}

/// Finds the next app slot to write the OTA update to.
///
/// We use an slot since it does not require lifetimes and is easier to handle.
// Tested with espflash md5 and espflash write-bin. Writing with SFTP seems to work fine.
async fn write_to_target(offset: u32, data: &[u8]) -> StorageResult<()> {
    let Some(fb) = flash::get_flash_n_buffer() else {
        error!("Flash storage not initialized");
        return Err(StorageError::InternalError);
    };
    let mut fb = fb.lock().await;

    let (storage, _buffer) = fb.split_ref_mut();

    // OtaUpdater is very particular. It needs a mut ref of a buffer of the exact size of the partition table.
    // This is why we create it here and did not reuse the buffer from fb.
    let mut buff_ota = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];

    let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_ota)
        .map_err(|e| {
            error!("Could not create OtaUpdater: {:?}", e);
            StorageError::InternalError
        })?;
    let (mut target_partition, part_type) = ota.next_partition().map_err(|e| {
        error!("Could not get next partition: {:?}", e);
        StorageError::InternalError
    })?;

    debug!("Flashing image to {:?}", part_type);

    debug!(
        "Writing data to target_partition at offset {}, with len {}",
        offset,
        data.len()
    );
    target_partition.write(offset, data).map_err(|e| {
        error!("Failed to write data to target_partition: {}", e);
        StorageError::WriteError
    })?;

    Ok(())
}

// TODO: Does not crash but the OTADATA partition is invalid on boot
/// The provided target slot will be marked as current and the image will be set as New so after
/// the reboot it will boot from it and be validated if the bootloader requires it.
///
/// We use a slot since it does not require lifetimes and is easier to handle.
async fn activate_next_ota_slot() -> StorageResult<()> {
    let Some(fb) = flash::get_flash_n_buffer() else {
        error!("Flash storage not initialized");
        return Err(StorageError::InternalError);
    };
    let mut fb = fb.lock().await;

    let (storage, _buffer) = fb.split_ref_mut();

    // OtaUpdater is very particular. It needs a mut ref of a buffer of the exact size of the partition table.
    // This is why we create it here and did not reuse the buffer from fb.
    let mut buff_ota = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];

    let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_ota)
        .map_err(|e| {
            error!("Could not create OtaUpdater: {:?}", e);
            StorageError::InternalError
        })?;

    ota.activate_next_partition().map_err(|e| {
        error!("Could not activate next partition: {:?}", e);
        StorageError::WriteError
    })?;
    ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::New)
        .map_err(|e| {
            error!("Could not set OTA image state to New: {:?}", e);
            StorageError::WriteError
        })?;

    Ok(())
}

// TODO: build bootloader with auto-rollback to avoid invalid images rendering the device unbootable
// TODO: Report bug in OtaImageState to esp
/// Validate the current OTA partition
///
/// Mark the current OTA slot as VALID - this is only needed if the bootloader was built with auto-rollback support.
/// The default pre-compiled bootloader in espflash is NOT.
///
pub async fn try_validating_current_ota_partition() -> StorageResult<()> {
    // Taken from [esp-hal ota_update example](https://github.com/esp-rs/esp-hal/examples/src/bin/ota_update.rs)
    let Some(fb) = flash::get_flash_n_buffer() else {
        error!("Flash storage not initialized");
        return Err(StorageError::InternalError);
    };
    let mut fb = fb.lock().await;

    let (storage, _buffer) = fb.split_ref_mut();

    // OtaUpdater is very particular. It needs a mut ref of a buffer of the exact size of the partition table.
    // This is why we create it here and did not reuse the buffer from fb.
    let mut buff_ota = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];

    let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_ota)
        .map_err(|e| {
            error!("Could not create OtaUpdater: {:?}", e);
            StorageError::InternalError
        })?;
    let current = ota.selected_partition().map_err(|e| {
        error!("Could not get selected partition: {:?}", e);
        StorageError::InternalError
    })?;

    debug!(
        "current image state {:?} (only relevant if the bootloader was built with auto-rollback support)",
        ota.current_ota_state()
    );
    debug!("currently selected partition {:?}", current);

    if let Ok(state) = ota.current_ota_state() {
        if state == esp_bootloader_esp_idf::ota::OtaImageState::New
            || state == esp_bootloader_esp_idf::ota::OtaImageState::PendingVerify
        {
            ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::Valid)
                .map_err(|e| {
                    error!("Could not set OTA image state to Valid: {:?}", e);
                    StorageError::WriteError
                })?;
            debug!("Changed state to VALID");
        }
    }

    Ok(())
}
