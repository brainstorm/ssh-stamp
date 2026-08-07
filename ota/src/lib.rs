// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! SFTP-based OTA update server for ssh-stamp.
//!
//! Receives firmware images over SFTP, validates the TLV header, writes
//! chunks to the OTA partition via [`OtaActions`](ssh_stamp_hal::OtaActions),
//! marks the partition bootable, and resets the device into the new image.
//!
//! The [`tlv`] module defines the TLV record format used by the `packer`
//! host utility and the on-device parser. The `packer` binary
//! (`ota/src/bin/packer.rs`) wraps a raw app binary into an `.otap` blob
//! with the required TLV header (OTA type, SHA-256 checksum, firmware size).
//!
//! This crate is `no_std` on embedded targets. The `std` feature gate and
//! `cfg(target_os = "none")` keep the SFTP server and handler modules
//! compiled out on the host, while the [`tlv`] module and [`OtaHeader`]
//! remain usable from both host and device code.

#![cfg_attr(not(test), no_std)]

/// Runs the OTA server taking care of reading OTA file metadata,
/// internal state, storage and target reset.
///
/// Entry point for this crate when used as an OTA server on the device.
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

/// Module defining TLV types and constants for OTA updates
///
/// Re-exporting this module for easier access from outside the crate: packer
pub mod tlv;

/// OTA Header structure and deserialization logic
///
/// Re-exporting Header for easier access from outside the crate: packer
pub use tlv::OtaHeader;

#[cfg(test)]
mod ota_tlv_tests {

    use crate::OtaHeader;
    use crate::tlv::*;
    use sunset::sshwire;

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
        for variant in &variants {
            let mut buffer = [0u8; MAX_TLV_SIZE as usize];
            let used = sshwire::write_ssh(&mut buffer, variant).expect("Failed to create SSH sink");

            // sunset 0.6's read_ssh also reports how many bytes it consumed.
            let (decoded, _) =
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

    #[test]
    fn target_chip_round_trip() {
        let variant = Tlv::TargetChip {
            chip: TargetChipName::try_from("esp32c6").unwrap(),
        };
        let mut buffer = [0u8; MAX_TLV_SIZE as usize];
        let used = sshwire::write_ssh(&mut buffer, &variant).expect("Failed to encode TLV");

        // type + length + "esp32c6"
        assert_eq!(used, 1 + 1 + 7);
        assert_eq!(buffer[0], TARGET_CHIP);
        assert_eq!(buffer[1], 7);

        let (decoded, _) =
            sshwire::read_ssh::<Tlv>(&buffer[..used], None).expect("Failed to decode TLV");
        match decoded {
            Tlv::TargetChip { chip } => assert_eq!(chip.as_str(), "esp32c6"),
            other => panic!("Decoded the wrong variant: {other:?}"),
        }
    }

    #[test]
    fn target_chip_is_serialized_right_after_ota_type() {
        // The whole point of the TLV is early rejection, so it must land in
        // the first bytes a device sees rather than after the checksum.
        let mut buffer = [0u8; 512];
        let used = OtaHeader::new(OTA_TYPE_VALUE_SSH_STAMP, &[7u8; 32], 2048, Some("esp32c6"))
            .serialize(&mut buffer);

        assert_eq!(buffer[0], OTA_TYPE);
        assert_eq!(buffer[6], TARGET_CHIP);

        let (header, consumed) =
            OtaHeader::deserialize(&buffer[..used]).expect("Failed to deserialize header");
        assert_eq!(consumed, used);
        assert_eq!(
            header.target_chip.as_ref().map(heapless::String::as_str),
            Some("esp32c6")
        );
        assert_eq!(header.sha256_checksum, Some([7u8; 32]));
        assert_eq!(header.firmware_blob_size, Some(2048));
    }

    #[test]
    fn header_without_target_chip_stays_readable() {
        // Images packed before the TLV existed must keep working.
        let mut buffer = [0u8; 512];
        let used =
            OtaHeader::new(OTA_TYPE_VALUE_SSH_STAMP, &[7u8; 32], 2048, None).serialize(&mut buffer);

        let (header, _) =
            OtaHeader::deserialize(&buffer[..used]).expect("Failed to deserialize header");
        assert_eq!(header.target_chip, None);
        assert_eq!(header.firmware_blob_size, Some(2048));
    }

    #[test]
    fn target_chip_before_ota_type_is_rejected() {
        let mut buffer = [0u8; 512];
        let mut offset = 0;

        let target = Tlv::TargetChip {
            chip: TargetChipName::try_from("esp32c6").unwrap(),
        };
        offset += sshwire::write_ssh(&mut buffer[offset..], &target)
            .expect("Failed to write Target Chip TLV");

        let ota_type_tlv = Tlv::OtaType {
            ota_type: OTA_TYPE_VALUE_SSH_STAMP,
        };
        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_type_tlv)
            .expect("Failed to write OTA Type TLV");

        assert!(OtaHeader::deserialize(&buffer[..offset]).is_err());
    }

    #[test]
    fn overlong_target_chip_is_rejected() {
        // Hand-rolled: the encoder cannot produce this, a hostile packer can.
        let mut buffer = [0u8; 512];
        buffer[0] = TARGET_CHIP;
        let too_long = MAX_TARGET_CHIP_LEN + 1;
        buffer[1] = u8::try_from(too_long).expect("test length fits in a u8");
        for slot in &mut buffer[2..=(2 + MAX_TARGET_CHIP_LEN)] {
            *slot = b'x';
        }
        let len = 2 + too_long;

        assert!(sshwire::read_ssh::<Tlv>(&buffer[..len], None).is_err());
    }

    #[test]
    fn non_utf8_target_chip_is_rejected() {
        let buffer = [TARGET_CHIP, 2, 0xff, 0xfe];
        assert!(sshwire::read_ssh::<Tlv>(&buffer, None).is_err());
    }

    // TODO: Test more error cases, such as incomplete TLVs
}
