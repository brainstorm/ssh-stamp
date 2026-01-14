// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

/// This structure is meant to wrap the media access for writing the OTA firmware
/// It will abstract the flash memory or other storage media so later we can implement it for different platforms
use crate::handler::OtaError;

struct OtaWriter {}

impl OtaWriter {
    pub fn new() -> Self {
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
