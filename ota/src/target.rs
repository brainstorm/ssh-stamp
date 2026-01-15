// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

// use esp_storage::FlashStorage;

use log::{debug, error, info, warn};
use storage::flash;

/// This structure is meant to wrap the media access for writing the OTA firmware
/// It will abstract the flash memory or other storage media so later we can implement it for different platforms
use crate::handler::OtaError;

use esp_bootloader_esp_idf::{
    ota::Slot,
    partitions::{self, DataPartitionSubType},
};

struct OtaWriter {}

impl OtaWriter {
    pub fn new() -> Self {
        // let mut flash = FlashStorage::new(FlashStorage::DEFAULT);
        // let mut buffer = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];
        // let pt = esp_bootloader_esp_idf::partitions::read_partition_table(
        //     &mut flash.get_mut(),
        //     &mut buffer,
        // )
        // .unwrap();

        // // List all partitions - this is just FYI
        // println!("Partitions:");
        // for i in 0..pt.len() {
        //     println!("{:?}", pt.get_partition(i));
        // }

        OtaWriter {}
    }

    pub fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), OtaError> {
        // Implement writing to flash memory or other storage media here
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<(), OtaError> {
        // Implement finalization logic here, such as verifying the written data
        Ok(())
    }
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
        &mut buffer[..partitions::PARTITION_TABLE_MAX_LEN],
    ) else {
        error!("Could not read partition table");
        return Err(());
    };

    // Find the OTA-data partition and show the currently active partition
    let Ok(Some(ota_data_part)) = pt.find_partition(
        esp_bootloader_esp_idf::partitions::PartitionType::Data(DataPartitionSubType::Ota),
    ) else {
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

    if current == Slot::None {
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
                warn!("The current ota image is marking is unknown");
            } // esp_bootloader_esp_idf::ota::OtaImageState::Undefined => {
              //     warn!("The current ota image is marked as Undefined");
              // }
        }
    }

    Ok(())
}
