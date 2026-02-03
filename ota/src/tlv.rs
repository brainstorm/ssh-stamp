// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

/// Module to define the tlv types for OTA metadata
///
/// Unsophisticated implementation of the ssh-stamp OTA TLV types using sshwire traits.
///
/// If you are looking into improving this, consider looking into [proto.rs](https://github.com/mkj/sunset/blob/8e5d20916cf7b29111b90e4d3b7bb7827c9be8e5/sftp/src/proto.rs)
/// for an example on how to automate the generation of protocols with macros
use log::{debug, error, info, warn};
use sunset::sshwire::{SSHDecode, SSHEncode, SSHSource, WireError};

use crate::tlv;

/// Type alias for OTA TLV type: The type field in the TLV structure will be an u8
pub type OtaTlvType = u8;
/// Type alias for OTA TLV length: The length field in the TLV structure will be an u8
pub type OtaTlvLen = u8;

// TODO: We could provide a new type for better debugging information
pub const OTA_TYPE_VALUE_SSH_STAMP: u32 = 0x73736873; // 'sshs' big endian in ASCII

pub const CHECKSUM_LEN: u32 = 32;
/// Maximum size for LTV (Length-Type-Value) entries in OTA metadata. Used during the reading of OTA parameters.
pub const MAX_TLV_SIZE: u32 = (core::mem::size_of::<OtaTlvType>()
    + core::mem::size_of::<OtaTlvLen>()
    + u8::MAX as usize) as u32; // type + length + value

/// Encodes the length and value of a sized values
fn enc_len_val<SE>(
    value: &SE,
    s: &mut dyn sunset::sshwire::SSHSink,
) -> sunset::sshwire::WireResult<()>
where
    SE: Sized + SSHEncode,
{
    (core::mem::size_of::<SE>() as OtaTlvLen).enc(s)?;
    value.enc(s)
}

/// Decodes and checks that the length of the value matches the expected size
///
/// Call it before decoding the actual value for simple types
fn dec_check_val_len<'de, S, SE>(s: &mut S) -> sunset::sshwire::WireResult<()>
where
    S: sunset::sshwire::SSHSource<'de>,
    SE: Sized,
{
    let val_len = OtaTlvLen::dec(s)?;
    if val_len != (core::mem::size_of::<SE>() as OtaTlvLen) {
        return Err(sunset::sshwire::WireError::PacketWrong);
    }
    Ok(())
}

// OTA TLV type defined values

pub const OTA_TYPE: OtaTlvType = 0;
pub const FIRMWARE_BLOB: OtaTlvType = 1;
pub const SHA256_CHECKSUM: OtaTlvType = 2;

/// OTA_TLV enum for OTA metadata LTV entries
/// This TLV does not capture length as it will be captured during parsing
/// Parsing will be done using sshwire types
#[derive(Debug)]
#[repr(u8)] // Must match the type of OtaTlvType
pub enum Tlv {
    /// Type of OTA update. This **MUST be the first Tlv**.
    /// For SSH Stamp, this must be OTA_FIRMWARE_BLOB_TYPE
    OtaType { ota_type: u32 },
    /// Expected SHA256 checksum of the firmware blob
    Sha256Checksum {
        checksum: [u8; CHECKSUM_LEN as usize],
    },
    /// Contains the length in bytes of the firmware blob.
    /// The firmware blob follows immediately after this TLV.
    ///
    /// This **MUST be the last Tlv**. What follows is the firmware blob. the length of the blob is the payload value.
    FirmwareBlob { size: u32 },
}

impl SSHEncode for Tlv {
    fn enc(&self, s: &mut dyn sunset::sshwire::SSHSink) -> sunset::sshwire::WireResult<()> {
        match self {
            Tlv::OtaType { ota_type } => {
                OTA_TYPE.enc(s)?;
                enc_len_val(ota_type, s)
            }
            Tlv::FirmwareBlob { size } => {
                FIRMWARE_BLOB.enc(s)?;
                enc_len_val(size, s)
            }
            Tlv::Sha256Checksum { checksum } => {
                SHA256_CHECKSUM.enc(s)?;
                enc_len_val(checksum, s)
            }
        }
    }
}

impl<'de> SSHDecode<'de> for Tlv {
    fn dec<S>(s: &mut S) -> sunset::sshwire::WireResult<Self>
    where
        S: sunset::sshwire::SSHSource<'de>,
    {
        OtaTlvType::dec(s).and_then(|tlv_type| match tlv_type {
            FIRMWARE_BLOB => {
                dec_check_val_len::<S, u32>(s)?;
                Ok(Tlv::FirmwareBlob { size: u32::dec(s)? })
            }
            SHA256_CHECKSUM => {
                if OtaTlvLen::dec(s)? != tlv::CHECKSUM_LEN as u8 {
                    return Err(sunset::sshwire::WireError::PacketWrong);
                }
                let mut checksum = [0u8; tlv::CHECKSUM_LEN as usize];
                checksum.iter_mut().for_each(|element| {
                    *element = u8::dec(s).unwrap_or(0);
                });
                Ok(Tlv::Sha256Checksum { checksum })
            }
            OTA_TYPE => {
                dec_check_val_len::<S, u32>(s)?;
                let ota_type = u32::dec(s)?;
                Ok(Tlv::OtaType { ota_type })
            }
            // To handle unknown TLVs, it consumes the announced len
            // and returns an UnknownVariant error
            _ => {
                warn!("Unknown TLV type encountered: {}. Skipping it", tlv_type);
                let len = OtaTlvLen::dec(s)?;
                s.take(len as usize)?; // Skip unknown TLV value
                Err(sunset::sshwire::WireError::UnknownPacket { number: tlv_type })
            }
        })
    }
}

/// An implementation of SSHSource based on [[sunset::sshwire::DecodeBytes]]
///
pub struct TlvsSource<'a> {
    remaining_buf: &'a [u8],
    ctx: sunset::packets::ParseContext,
    used: usize,
}

impl<'a> TlvsSource<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            remaining_buf: buf,
            ctx: sunset::packets::ParseContext::default(),
            used: 0,
        }
    }

    pub fn used(&self) -> usize {
        self.used
    }
    /// Puts bytes in the tlv_holder and updates current_len until an OTA TLV enum variant can be decoded
    ///
    /// Even if it fails, it adds bytes to the tlv_holder and updates current_len accordingly
    /// so more data can be added later to complete the TLV
    ///
    /// If more data is required, it returns WireError::RanOut
    /// If successful, it returns Ok(()) and a dec
    // TODO: Add test for RanOut and acomplete TLV
    pub fn try_taking_bytes_for_tlv(
        &mut self,
        tlv_holder: &mut [u8],
        current_len: &mut usize,
    ) -> Result<(), WireError> {
        if *current_len
            < core::mem::size_of::<tlv::OtaTlvType>() + core::mem::size_of::<tlv::OtaTlvLen>()
        {
            let needed = core::mem::size_of::<tlv::OtaTlvType>()
                + core::mem::size_of::<tlv::OtaTlvLen>()
                - *current_len;
            debug!("Adding {} bytes to have up to TLV type and length", needed);
            let to_read = core::cmp::min(needed, self.remaining());
            let type_len_bytes = self.take(to_read)?;
            tlv_holder[*current_len..*current_len + to_read].copy_from_slice(type_len_bytes);
            *current_len += to_read;
            if needed < to_read {
                info!("Will get more data to complete TLV type/length");
                return Err(WireError::RanOut);
            }
        }

        let slice_len_start = core::mem::size_of::<tlv::OtaTlvType>();
        let slice_value_start =
            core::mem::size_of::<tlv::OtaTlvType>() + core::mem::size_of::<tlv::OtaTlvLen>();
        if *current_len >= slice_value_start {
            // try reading bytes to complete the value
            let val_len = tlv::OtaTlvLen::from_be_bytes(
                tlv_holder[slice_len_start..slice_value_start]
                    .try_into()
                    .unwrap(),
            ) as usize;
            debug!(
                "value length: {}, Source remaining bytes: {}",
                val_len,
                self.remaining()
            );

            let needed = val_len + slice_value_start - *current_len;
            let to_read = needed.min(self.remaining());

            let needed_type_len_bytes = self.take(to_read)?;
            tlv_holder[*current_len..*current_len + to_read].copy_from_slice(needed_type_len_bytes);
            *current_len += to_read;
            if needed < to_read {
                info!("Will get more data to complete TLV type/length");
                return Err(WireError::RanOut);
            }
        }
        Ok(())
    }
}

impl<'de> SSHSource<'de> for TlvsSource<'de> {
    fn take(&mut self, len: usize) -> sunset::sshwire::WireResult<&'de [u8]> {
        if len > self.remaining_buf.len() {
            return Err(sunset::sshwire::WireError::RanOut);
        }
        let t;
        (t, self.remaining_buf) = self.remaining_buf.split_at(len);
        self.used += len;
        Ok(t)
    }

    fn remaining(&self) -> usize {
        self.remaining_buf.len()
    }

    fn ctx(&mut self) -> &mut sunset::packets::ParseContext {
        &mut self.ctx
    }
}

/// Header struct for OTA file header processing
///
/// This struct holds the metadata that will be used to validate the OTA file prior to applying the update.
///
/// The fields serialisation and deserialization
#[derive(Debug)]
pub struct OtaHeader {
    // Not part of the header data
    // hasher: sha2::Sha256,
    /// Type of OTA update being processed. Used for screening incorrect ota blobs quickly
    pub(crate) ota_type: Option<u32>,
    /// Total size of the firmware being downloaded, if known
    pub(crate) firmware_blob_size: Option<u32>,
    /// Expected sha256 checksum of the firmware, if provided
    pub sha256_checksum: Option<[u8; tlv::CHECKSUM_LEN as usize]>,
}

impl OtaHeader {
    /// Creates a new OTA header with the provided parameters
    ///
    /// Used during packing of OTA files. Therefore, not needed in the embedded side.
    #[cfg(not(target_os = "none"))]
    pub fn new(ota_type: u32, sha256_checksum: &[u8], firmware_blob_size: u32) -> Self {
        // TODO: Check that the sha256_checksum length is correct: 32 bytes
        let mut checksum_array = [0u8; tlv::CHECKSUM_LEN as usize];
        checksum_array.copy_from_slice(sha256_checksum);
        Self {
            ota_type: Some(ota_type),
            firmware_blob_size: Some(firmware_blob_size),
            sha256_checksum: Some(checksum_array),
        }
    }

    /// Serializes the OTA header into the provided buffer
    ///
    /// Returns the number of bytes written to the buffer
    // #[cfg(not(target_os = "none"))] // Maybe I should remove this from embedded side as well
    pub fn serialize(&self, buf: &mut [u8]) -> usize {
        let mut offset = 0;
        if let Some(ota_type) = self.ota_type {
            let tlv = tlv::Tlv::OtaType { ota_type };
            let used = sunset::sshwire::write_ssh(&mut buf[offset..], &tlv)
                .expect("Failed to serialize OTA Type TLV");
            offset += used;
        }
        if let Some(checksum) = &self.sha256_checksum {
            let tlv = tlv::Tlv::Sha256Checksum {
                checksum: *checksum,
            };
            let used = sunset::sshwire::write_ssh(&mut buf[offset..], &tlv)
                .expect("Failed to serialize SHA256 Checksum TLV");
            offset += used;
        }
        if let Some(size) = self.firmware_blob_size {
            let tlv = tlv::Tlv::FirmwareBlob { size };
            let used = sunset::sshwire::write_ssh(&mut buf[offset..], &tlv)
                .expect("Failed to serialize Firmware Blob TLV");
            offset += used;
        }
        offset
    }

    /// Deserializes an OTA header from the provided buffer
    ///
    /// This approach requires that the whole header is contained in the buffer. An incomplete
    /// header will result in unpopulated fields.
    pub fn deserialize(buf: &[u8]) -> Result<(Self, usize), sunset::sshwire::WireError> {
        let mut source = tlv::TlvsSource::new(buf);
        let mut ota_type = None;
        let mut firmware_blob_size = None;
        let mut sha256_checksum = None;

        while source.remaining() > 0 {
            match tlv::Tlv::dec(&mut source) {
                Err(sunset::sshwire::WireError::UnknownPacket { number }) => {
                    warn!(
                        "Unknown packet type encountered: {}. TLV skipping it and continuing",
                        number
                    );
                    // Unknown TLV was skipped already in the decoder
                    continue;
                }
                Err(e) => {
                    return Err(e);
                }
                Ok(tlv) => {
                    match tlv {
                        tlv::Tlv::OtaType { ota_type: ot } => {
                            ota_type = Some(ot);
                        }
                        tlv::Tlv::Sha256Checksum { checksum } => {
                            Self::check_ota_is_first_tlv(ota_type)?;
                            sha256_checksum = Some(checksum);
                        }
                        tlv::Tlv::FirmwareBlob { size } => {
                            Self::check_ota_is_first_tlv(ota_type)?;
                            firmware_blob_size = Some(size);
                            // After firmware blob, there shall be no more tlvs and the
                            // actual blob follows. Therefore we stop reading here
                            break;
                        }
                    }
                }
            }
        }

        Ok((
            Self {
                ota_type,
                firmware_blob_size,
                sha256_checksum,
            },
            source.used(),
        ))
    }

    fn check_ota_is_first_tlv(ota_type: Option<u32>) -> Result<(), WireError> {
        match ota_type.is_none() {
            true => {
                error!("SHA256 Checksum TLV encountered before OTA Type TLV. Ignoring it");
                Err(sunset::sshwire::WireError::PacketWrong)
            }
            false => Ok(()),
        }
    }
}
