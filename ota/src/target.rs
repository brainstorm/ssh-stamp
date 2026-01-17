// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

// use esp_storage::FlashStorage;

use log::{debug, error, info, warn};
use storage::flash;

/// This structure is meant to wrap the media access for writing the OTA firmware
/// It will abstract the flash memory or other storage media so later we can implement it for different platforms
use crate::handler::OtaError;

use esp_bootloader_esp_idf;

use embedded_storage::Storage;
use esp_hal::system;

#[derive(Debug, Copy, Clone)]
pub(crate) struct OtaWriter {
    target_slot: esp_bootloader_esp_idf::ota::Slot,
}

impl OtaWriter {
    /// Creates a new OtaWriter for the given target OTA slot.
    ///
    /// To obtain a target OTA slot use [get_next_app_slot]
    pub(crate) fn new(target_slot: esp_bootloader_esp_idf::ota::Slot) -> Self {
        OtaWriter { target_slot }
    }
    // TODO: Not tested. May crash!
    /// Writes data to the target OTA partition at the given offset.
    pub(crate) async fn write(&self, offset: u32, data: &[u8]) -> Result<(), OtaError> {
        write_to_target(self.target_slot, offset, data).await
    }
    // TODO: Not tested. May crash!
    /// Finalizes the OTA update by setting the target slot as current.
    pub async fn finalize(&mut self) -> Result<(), OtaError> {
        set_current(self.target_slot).await?;
        system::software_reset(); // TODO: Not the right place. I would need to signal the main app to reboot after closing the SFTP session
    }
}

// TODO: Not tested. Unlikely to crash but might be refactored
/// Finds the next app slot to write the OTA update to.
///
/// We use an slot since it does not require lifetimes and is easier to handle, but it does create
/// overhead in the [write_to_target] and [set_current] functions.
pub async fn get_next_app_slot() -> Result<esp_bootloader_esp_idf::ota::Slot, OtaError> {
    let mut fb = flash::get_flash_n_buffer().lock().await;
    let (storage, buffer) = fb.split_ref_mut();
    let Ok(pt) = esp_bootloader_esp_idf::partitions::read_partition_table(
        storage,
        &mut buffer[..esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN],
    ) else {
        error!("Could not read partition table");
        return Err(OtaError::WriteError);
    };

    // Find the OTA-data partition and show the currently active partition
    let Ok(Some(ota_data_part)) =
        pt.find_partition(esp_bootloader_esp_idf::partitions::PartitionType::Data(
            esp_bootloader_esp_idf::partitions::DataPartitionSubType::Ota,
        ))
    else {
        error!("Could not find OTA data partition");
        return Err(OtaError::WriteError);
    };

    let mut ota_data_part = ota_data_part.as_embedded_storage(storage);
    debug!("Found ota data");

    let Ok(mut ota) = esp_bootloader_esp_idf::ota::Ota::new(&mut ota_data_part) else {
        error!("Could not initialize OTA handler");
        return Err(OtaError::WriteError);
    };
    let Ok(current_slot) = ota.current_slot() else {
        error!("Could not obtain the next ota slot");
        return Err(OtaError::WriteError);
    };
    let next_slot = current_slot.next();
    debug!("Next ota slot: {:?}", next_slot);

    Ok(next_slot)
}
// TODO: Not tested. May crash!!!
/// Finds the next app slot to write the OTA update to.
///
/// We use an slot since it does not require lifetimes and is easier to handle.
async fn write_to_target(
    target_slot: esp_bootloader_esp_idf::ota::Slot,
    offset: u32,
    data: &[u8],
) -> Result<(), OtaError> {
    let mut fb = flash::get_flash_n_buffer().lock().await;
    let (storage, buffer) = fb.split_ref_mut();
    let Ok(pt) = esp_bootloader_esp_idf::partitions::read_partition_table(
        storage,
        &mut buffer[..esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN],
    ) else {
        error!("Could not read partition table");
        return Err(OtaError::WriteError);
    };
    debug!("Resolving target_partition");
    // Unfortunately, this is pretty convoluted way to get the target partition
    let target_partition = match target_slot {
        esp_bootloader_esp_idf::ota::Slot::None =>
        // None is FACTORY if present, OTA0 otherwise
        {
            pt.find_partition(esp_bootloader_esp_idf::partitions::PartitionType::App(
                esp_bootloader_esp_idf::partitions::AppPartitionSubType::Factory,
            ))
            .or_else(|_| {
                pt.find_partition(esp_bootloader_esp_idf::partitions::PartitionType::App(
                    esp_bootloader_esp_idf::partitions::AppPartitionSubType::Ota0,
                ))
            })
        }
        esp_bootloader_esp_idf::ota::Slot::Slot0 => {
            pt.find_partition(esp_bootloader_esp_idf::partitions::PartitionType::App(
                esp_bootloader_esp_idf::partitions::AppPartitionSubType::Ota0,
            ))
        }
        esp_bootloader_esp_idf::ota::Slot::Slot1 => {
            pt.find_partition(esp_bootloader_esp_idf::partitions::PartitionType::App(
                esp_bootloader_esp_idf::partitions::AppPartitionSubType::Ota1,
            ))
        }
    };
    let Ok(Some(target_partition)) = target_partition else {
        error!("Could not find target OTA partition");
        return Err(OtaError::WriteError);
    };

    debug!("Resolving target_partition storage");
    let mut target_partition = target_partition.as_embedded_storage(storage);

    debug!(
        "Writing data to target_partition at offset {}, with len {}",
        offset,
        data.len()
    );
    target_partition.write(offset, data).map_err(|e| {
        error!("Failed to write data to target_partition: {}", e);
        OtaError::WriteError
    })?;
    Ok(())
}

// TODO: Not tested. May crash!!!
/// The provided target slot will be marked as current and the image will be set as New so after
/// the reboot it will boot from it and be validated if the bootloader requires it.
///
/// We use an slot since it does not require lifetimes and is easier to handle.
async fn set_current(target_slot: esp_bootloader_esp_idf::ota::Slot) -> Result<(), OtaError> {
    let mut fb = flash::get_flash_n_buffer().lock().await;
    let (storage, buffer) = fb.split_ref_mut();
    let Ok(pt) = esp_bootloader_esp_idf::partitions::read_partition_table(
        storage,
        &mut buffer[..esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN],
    ) else {
        error!("Could not read partition table");
        return Err(OtaError::WriteError);
    };

    // Find the OTA-data partition and show the currently active partition
    let Ok(Some(ota_data_part)) =
        pt.find_partition(esp_bootloader_esp_idf::partitions::PartitionType::Data(
            esp_bootloader_esp_idf::partitions::DataPartitionSubType::Ota,
        ))
    else {
        error!("Could not find OTA data partition");
        return Err(OtaError::WriteError);
    };

    let mut ota_data_part = ota_data_part.as_embedded_storage(storage);
    debug!("Found ota data. Creating handle to modify properties");
    let mut ota = esp_bootloader_esp_idf::ota::Ota::new(&mut ota_data_part)
        .map_err(|_| OtaError::WriteError)?;
    info!("Setting current ota slot to {:?}", target_slot);
    ota.set_current_slot(target_slot)
        .map_err(|_| OtaError::WriteError)?;
    debug!("setting current ota state to New");
    ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::New)
        .map_err(|_| OtaError::WriteError)
}

// TODO: build bootloader with auto-rollback to avoid invalid images rendering the device unbootable
// TODO: Report bug in OtaImageState to esp
/// Validate the current OTA partition
///
/// Mark the current OTA slot as VALID - this is only needed if the bootloader was built with auto-rollback support.
/// The default pre-compiled bootloader in espflash is NOT.
///
/// Taken from [esp-rs ota_update example](https://github.com/esp-rs/esp-hal/blob/99042a7d60388580459eab6fe0d10e2f89d6ab6c/examples/src/bin/ota_update.rs)
pub async fn validate_current_ota_partition() -> Result<(), ()> {
    let mut fb = flash::get_flash_n_buffer().lock().await;
    let (storage, buffer) = fb.split_ref_mut();
    let Ok(pt) = esp_bootloader_esp_idf::partitions::read_partition_table(
        storage,
        &mut buffer[..esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN],
    ) else {
        error!("Could not read partition table");
        return Err(());
    };

    // Find the OTA-data partition and show the currently active partition
    let Ok(Some(ota_data_part)) =
        pt.find_partition(esp_bootloader_esp_idf::partitions::PartitionType::Data(
            esp_bootloader_esp_idf::partitions::DataPartitionSubType::Ota,
        ))
    else {
        error!("Could not find OTA data partition");
        return Err(());
    };

    let mut ota_data_part = ota_data_part.as_embedded_storage(storage);
    debug!("Found ota data");

    let Ok(mut ota) = esp_bootloader_esp_idf::ota::Ota::new(&mut ota_data_part) else {
        error!("Could not initialize OTA handler");
        return Err(());
    };

    let Ok(current) = ota.current_slot() else {
        error!("Could not obtain the current ota slot");
        return Err(());
    };

    let Ok(ota_image_state) = ota.current_ota_state() else {
        error!("Could not obtain the ota image state");
        return Err(());
    };

    if current == esp_bootloader_esp_idf::ota::Slot::None {
        error!("Current ota slot is None, cannot validate");
        return Err(());
    }

    if ota_image_state == esp_bootloader_esp_idf::ota::OtaImageState::Valid {
        info!("current {:?} ota partition is already valid", current);
        return Ok(());
    }

    if ota_image_state == esp_bootloader_esp_idf::ota::OtaImageState::New
        || ota_image_state == esp_bootloader_esp_idf::ota::OtaImageState::PendingVerify
    {
        let Ok(()) = ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::Valid)
        else {
            error!("Could not set the ota partition state to valid");
            return Err(());
        };
        info!("current {:?} ota partition state set to valid", current);
    } else {
        warn!("Current slot cannot be validated, no action taken");
        match ota_image_state {
            esp_bootloader_esp_idf::ota::OtaImageState::Valid => {
                warn!("The current ota image is marked as Valid");
            }
            esp_bootloader_esp_idf::ota::OtaImageState::New => {
                warn!("The current ota image is marked as New");
            }
            esp_bootloader_esp_idf::ota::OtaImageState::PendingVerify => {
                warn!("The current ota image is marked as PendingVerify");
            }
            esp_bootloader_esp_idf::ota::OtaImageState::Invalid => {
                warn!("The current ota image is marked as invalid");
            }
            esp_bootloader_esp_idf::ota::OtaImageState::Aborted => {
                warn!("The current ota image is marked as aborted");
            }
            // TODO: Report the crash? Crash: Exception 'Load access fault' mepc=0x4205cc54, mtval=0xfa3a8e38
            _ => {
                warn!("The current ota image marking is unknown");
            } // esp_bootloader_esp_idf::ota::OtaImageState::Undefined => {
              //     warn!("The current ota image is marked as Undefined");
              // }
        }
    }

    Ok(())
}
