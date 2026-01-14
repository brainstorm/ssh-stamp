// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

/// Module to define the tlv types for OTA metadata
///
/// Unsophisticated implementation of the ssh-stamp OTA TLV types using sshwire traits.
///
/// If you are looking into improving this, consider looking into [proto.rs](https://github.com/mkj/sunset/blob/8e5d20916cf7b29111b90e4d3b7bb7827c9be8e5/sftp/src/proto.rs)
/// for an example on how to automate the generation of protocols with macros
use log::{debug, info, warn};
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
    + u8::max_value() as usize) as u32; // type + length + value

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
    /// Type of OTA update. This MUST be the first Tlv.
    /// For SSH Stamp, this must be OTA_FIRMWARE_BLOB_TYPE
    OtaType { ota_type: u32 },
    /// Expected SHA256 checksum of the firmware blob
    Sha256Checksum {
        checksum: [u8; CHECKSUM_LEN as usize],
    },
    /// This MUST be the last Tlv. What follows is the firmware blob. the length of the blob is the payload value.
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
            info!(
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
