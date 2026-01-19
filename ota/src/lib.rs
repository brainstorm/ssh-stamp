#![cfg_attr(not(test), no_std)]
// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

/// Runs the ota server taking care of reading ota file metadata,
/// internal state, storage and target reset
///
/// Entry point for this crate when used as an OTA server
#[cfg(target_os = "none")]
pub use sftpserver::run_ota_server;
/// Module handling OTA update metadata and header parsing
///
/// It will be called from the sftpserver module to handle the OTA update process
#[cfg(target_os = "none")]
mod handler;
/// Module implementing the OTA SFTP server
#[cfg(target_os = "none")]
mod sftpserver;
/// Defining the target hardware abstraction for OTA updates
///
/// It heavily relies on esp-bootloader-esp-idf crate for handling the partitions and OTA slots
/// as it is described in the [esp-rs ota update example code](https://github.com/esp-rs/esp-hal/blob/99042a7d60388580459eab6fe0d10e2f89d6ab6c/examples/src/bin/ota_update.rs)
#[cfg(target_os = "none")]
mod target;

#[cfg(target_os = "none")]
pub use target::try_validating_current_ota_partition;
/// Module defining TLV types and constants for OTA updates
///
/// Re-exporting this module for easier access from outside the crate: ota-packer
pub mod tlv;

/// OTA Header structure and deserialization logic
///
/// Re-exporting Header for easier access from outside the crate: ota-packer
pub use tlv::OtaHeader;

#[cfg(test)]
mod ota_tlv_tests {

    use crate::OtaHeader;
    use crate::tlv::*;
    use sunset::sshwire::{self, SSHDecode, SSHEncode};

    #[test]
    fn test_ota_tlv_round_trip() {
        let variants = [
            Tlv::FirmwareBlob { size: 1024 },
            Tlv::Sha256Checksum {
                checksum: [
                    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                    23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
                ],
            },
            Tlv::OtaType {
                ota_type: OTA_TYPE_VALUE_SSH_STAMP,
            },
        ];
        for variant in variants.iter() {
            let mut buffer = [0u8; MAX_TLV_SIZE as usize];
            let used = sshwire::write_ssh(&mut buffer, variant).expect("Failed to create SSH sink");

            let decoded =
                sshwire::read_ssh::<Tlv>(&buffer[..used], None).expect("Failed to decode TLV");
            match (variant, decoded) {
                (Tlv::FirmwareBlob { size: s1 }, Tlv::FirmwareBlob { size: s2 }) => {
                    assert_eq!(s1, &s2);
                }
                (Tlv::Sha256Checksum { checksum: c1 }, Tlv::Sha256Checksum { checksum: c2 }) => {
                    assert_eq!(c1, &c2);
                }
                (Tlv::OtaType { ota_type: o1 }, Tlv::OtaType { ota_type: o2 }) => {
                    assert_eq!(o1, &o2);
                }
                _ => panic!("Decoded variant does not match original"),
            }
        }
    }

    #[test]
    fn deserializing_full_header() {
        let mut buffer = [0u8; 512];
        let mut offset = 0;

        let ota_type_tlv = Tlv::OtaType {
            ota_type: OTA_TYPE_VALUE_SSH_STAMP,
        };
        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_type_tlv)
            .expect("Failed to write OTA Type TLV");

        let ota_checksum = Tlv::Sha256Checksum {
            checksum: [
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 32,
            ],
        };

        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_checksum)
            .expect("Failed to write SHA256 Checksum TLV");

        let firmware_blob_tlv = Tlv::FirmwareBlob { size: 2048 };
        offset += sshwire::write_ssh(&mut buffer[offset..], &firmware_blob_tlv)
            .expect("Failed to write Firmware Blob TLV");

        let (header, _) =
            OtaHeader::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

        assert_eq!(header.ota_type, Some(OTA_TYPE_VALUE_SSH_STAMP));
        assert_eq!(header.firmware_blob_size, Some(2048));
        assert_eq!(
            header.sha256_checksum,
            Some([
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 32,
            ])
        );
    }

    #[test]
    fn tlvs_after_firmware_blob_are_ignored() {
        let mut buffer = [0u8; 512];
        let mut offset = 0;

        let ota_type_tlv = Tlv::OtaType {
            ota_type: OTA_TYPE_VALUE_SSH_STAMP,
        };
        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_type_tlv)
            .expect("Failed to write OTA Type TLV");

        let firmware_blob_tlv = Tlv::FirmwareBlob { size: 2048 };
        offset += sshwire::write_ssh(&mut buffer[offset..], &firmware_blob_tlv)
            .expect("Failed to write Firmware Blob TLV");

        // After firmware_blob. Will not be deserialised
        let ota_checksum = Tlv::Sha256Checksum {
            checksum: [
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 32,
            ],
        };
        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_checksum)
            .expect("Failed to write SHA256 Checksum TLV");

        let (header, _) =
            OtaHeader::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

        assert_eq!(header.ota_type, Some(OTA_TYPE_VALUE_SSH_STAMP));
        assert_eq!(header.firmware_blob_size, Some(2048));
        assert_eq!(header.sha256_checksum, None);
    }

    #[test]
    fn ota_type_must_be_first_tlv() {
        let mut buffer = [0u8; 512];
        let mut offset = 0;

        let ota_checksum = Tlv::Sha256Checksum {
            checksum: [
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 32,
            ],
        };

        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_checksum)
            .expect("Failed to write SHA256 Checksum TLV");

        let ota_type = Tlv::OtaType {
            ota_type: OTA_TYPE_VALUE_SSH_STAMP,
        };
        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_type)
            .expect("Failed to write OTA Type TLV");

        let firmware_blob_tlv = Tlv::FirmwareBlob { size: 2048 };
        offset += sshwire::write_ssh(&mut buffer[offset..], &firmware_blob_tlv)
            .expect("Failed to write Firmware Blob TLV");

        assert!(OtaHeader::deserialize(&buffer[..offset]).is_err());
    }

    #[test]
    fn deserializing_header_missing_firmware_blob() {
        let mut buffer = [0u8; 512];
        let mut offset = 0;

        let ota_type_tlv = Tlv::OtaType {
            ota_type: OTA_TYPE_VALUE_SSH_STAMP,
        };
        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_type_tlv)
            .expect("Failed to write OTA Type TLV");

        let ota_checksum = Tlv::Sha256Checksum {
            checksum: [
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 32,
            ],
        };

        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_checksum)
            .expect("Failed to write SHA256 Checksum TLV");

        let (header, _) =
            OtaHeader::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

        assert_eq!(header.ota_type, Some(OTA_TYPE_VALUE_SSH_STAMP));
        assert_eq!(header.firmware_blob_size, None);
        assert_eq!(
            header.sha256_checksum,
            Some([
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 32,
            ])
        );
    }

    #[test]
    fn skipping_unknown_tlv() {
        let mut buffer = [0u8; 512];
        let mut offset = 0;

        let ota_type_tlv = Tlv::OtaType {
            ota_type: OTA_TYPE_VALUE_SSH_STAMP,
        };
        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_type_tlv)
            .expect("Failed to write OTA Type TLV");

        // Manually generating a valid unknown type
        let unknown_type: OtaTlvType = 99;
        let unknown_type_len: OtaTlvLen = 4;
        let unknown_value: [u8; 4] = [10, 20, 30, 40];

        offset += sshwire::write_ssh(&mut buffer[offset..], &unknown_type)
            .expect("Failed to write unknown TLV type");
        offset += sshwire::write_ssh(&mut buffer[offset..], &unknown_type_len)
            .expect("Failed to write unknown TLV length");
        offset += sshwire::write_ssh(&mut buffer[offset..], &unknown_value)
            .expect("Failed to write unknown TLV value");

        let firmware_blob_tlv = Tlv::FirmwareBlob { size: 2048 };
        let used = sshwire::write_ssh(&mut buffer[offset..], &firmware_blob_tlv)
            .expect("Failed to write Firmware Blob TLV");
        offset += used;

        let (header, _) =
            OtaHeader::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

        assert_eq!(header.ota_type, Some(OTA_TYPE_VALUE_SSH_STAMP));
        assert_eq!(header.firmware_blob_size, Some(2048));
        assert_eq!(header.sha256_checksum, None);
    }

    // TODO: Test more error cases, such as incomplete TLVs
}
