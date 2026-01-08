#![cfg_attr(not(test), no_std)]
// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later
use log::{debug, error, warn};
use sunset::sshwire::{BinString, SSHDecode, WireError};
use sunset_async::ChanInOut;
use sunset_sftp::{
    SftpHandler,
    handles::OpaqueFileHandle,
    protocol::{FileHandle, Filename, NameEntry, PFlags},
    server::SftpServer,
};

use core::hash::Hasher;
use rustc_hash::FxHasher;
use sha2::Digest;

pub async fn run_ota_server(stdio: ChanInOut<'_>) -> Result<(), sunset::Error> {
    // Placeholder for OTA server logic
    // This function would handle the SFTP session and perform OTA updates
    debug!("SFTP not implemented");
    let mut buffer_in = [0u8; 512];
    let mut request_buffer = [0u8; 512];

    match {
        let mut file_server = SftpOtaServer::new();

        SftpHandler::<OtaOpaqueFileHandle, SftpOtaServer<OtaOpaqueFileHandle>, 512>::new(
            &mut file_server,
            &mut request_buffer,
        )
        .process_loop(stdio, &mut buffer_in)
        .await?;

        Ok::<_, sunset::Error>(())
    } {
        Ok(_) => {
            debug!("sftp server loop finished gracefully");
            Ok(())
        }
        Err(e) => {
            debug!("sftp server loop finished with an error: {}", &e);
            Err(e)
        }
    }
}

/// This length is chosen to keep the file handle small
/// while still providing a reasonable level of uniqueness.
/// We are not expecting more than one OTA operation at a time.
const HASH_LEN: usize = 4;
/// OtaOpaqueFileHandle for OTA SFTP server
///
/// Minimal implementation of an opaque file handle with a tiny hash
#[derive(Hash, Debug, Eq, PartialEq, Clone)]
struct OtaOpaqueFileHandle {
    // Define fields as needed for OTA file handle
    tiny_hash: [u8; HASH_LEN],
}

impl OpaqueFileHandle for OtaOpaqueFileHandle {
    fn new(seed: &str) -> Self {
        let mut hasher = FxHasher::default();
        hasher.write(seed.as_bytes());
        OtaOpaqueFileHandle {
            tiny_hash: (hasher.finish() as u32).to_be_bytes(),
        }
    }

    fn try_from(file_handle: &FileHandle<'_>) -> sunset::sshwire::WireResult<Self> {
        if !file_handle
            .0
            .0
            .len()
            .eq(&core::mem::size_of::<OtaOpaqueFileHandle>())
        {
            return Err(WireError::BadString);
        }

        let mut tiny_hash = [0u8; HASH_LEN];
        tiny_hash.copy_from_slice(file_handle.0.0);
        Ok(OtaOpaqueFileHandle { tiny_hash })
    }

    fn into_file_handle(&self) -> FileHandle<'_> {
        FileHandle(BinString(&self.tiny_hash))
    }
}

/// SFTP server implementation for OTA updates
///
/// This struct implements the SftpServer trait for handling OTA updates over SFTP
/// For now, all methods log an error and return unsupported operation as this is a placeholder
struct SftpOtaServer<T> {
    // Add fields as necessary for OTA server state
    file_handle: Option<T>,
    write_permission: bool,
    processor: UpdateProcessor,
}

impl<T> SftpOtaServer<T> {
    pub fn new() -> Self {
        Self {
            // Initialize fields as necessary
            file_handle: None,
            write_permission: false,
            processor: UpdateProcessor::new(),
        }
    }
}

impl<'a, T: OpaqueFileHandle> SftpServer<'a, T> for SftpOtaServer<T> {
    fn open(
        &'_ mut self,
        path: &str,
        mode: &sunset_sftp::protocol::PFlags,
    ) -> sunset_sftp::server::SftpOpResult<T> {
        if self.file_handle.is_none() {
            let num_mode = u32::from(mode);

            self.write_permission = num_mode & u32::from(&PFlags::SSH_FXF_WRITE) > 0
                || num_mode & u32::from(&PFlags::SSH_FXF_APPEND) > 0
                || num_mode & u32::from(&PFlags::SSH_FXF_CREAT) > 0;

            let handle = T::new(path);
            self.file_handle = Some(handle.clone());
            log::info!(
                "SftpServer Open operation: path = {:?}, write_permission = {:?}, handle = {:?}",
                path,
                self.write_permission,
                &handle
            );
            return Ok(handle);
        } else {
            log::error!(
                "SftpServer Open operation failed: already writing OTA, path = {:?}, attrs = {:?}",
                path,
                mode
            );
            return Err(sunset_sftp::protocol::StatusCode::SSH_FX_PERMISSION_DENIED);
        }
    }

    fn close(&mut self, handle: &T) -> sunset_sftp::server::SftpOpResult<()> {
        if let Some(current_handle) = &self.file_handle {
            if current_handle == handle {
                log::info!(
                    "SftpServer Close operation for OTA completed: handle = {:?}",
                    handle
                );
                self.file_handle = None;
                self.write_permission = false;
                return Ok(());
            } else {
                log::warn!(
                    "SftpServer Close operation failed: handle mismatch = {:?}",
                    handle
                );
                return Err(sunset_sftp::protocol::StatusCode::SSH_FX_FAILURE);
            }
        } else {
            log::warn!(
                "SftpServer Close operation granted on untracked handle: {:?}",
                handle
            );
            return Ok(()); // TODO: Handle close properly. You will need this for the OTA server
        }
    }

    fn read<const N: usize>(
        &mut self,
        opaque_file_handle: &T,
        offset: u64,
        len: u32,
        _reply: &mut sunset_sftp::server::ReadReply<'_, N>,
    ) -> impl core::future::Future<Output = sunset_sftp::error::SftpResult<()>> {
        async move {
            log::error!(
                "SftpServer Read operation not defined: handle = {:?}, offset = {:?}, len = {:?}",
                opaque_file_handle,
                offset,
                len
            );
            Err(sunset_sftp::error::SftpError::FileServerError(
                sunset_sftp::protocol::StatusCode::SSH_FX_OP_UNSUPPORTED,
            ))
        }
    }

    fn write(
        &mut self,
        opaque_file_handle: &T,
        offset: u64,
        buf: &[u8],
    ) -> sunset_sftp::server::SftpOpResult<()> {
        if let Some(current_handle) = &self.file_handle {
            if current_handle == opaque_file_handle {
                if !self.write_permission {
                    log::warn!(
                        "SftpServer Write operation denied: no write permission for handle = {:?}",
                        opaque_file_handle
                    );
                    return Err(sunset_sftp::protocol::StatusCode::SSH_FX_PERMISSION_DENIED);
                }
                log::info!(
                    "SftpServer Write operation for OTA: handle = {:?}, offset = {:?}, buf_len = {:?}",
                    opaque_file_handle,
                    offset,
                    buf.len()
                );
                // Here you would add the logic to write the buffer to the OTA update mechanism
                return Ok(());
            }
        }

        log::warn!(
            "SftpServer Write operation failed: handle mismatch = {:?}",
            opaque_file_handle
        );
        return Err(sunset_sftp::protocol::StatusCode::SSH_FX_FAILURE);
    }

    fn opendir(&mut self, dir: &str) -> sunset_sftp::server::SftpOpResult<T> {
        let handle = T::new(dir);
        log::info!(
            "SftpServer OpenDir: dir = {:?}. Returning {:?}",
            dir,
            &handle
        );
        Ok(handle) // TODO: Store handle and use salt
    }

    fn readdir<const N: usize>(
        &mut self,
        _opaque_dir_handle: &T,
        _reply: &mut sunset_sftp::server::DirReply<'_, N>,
    ) -> impl core::future::Future<Output = sunset_sftp::server::SftpOpResult<()>> {
        async move {
            log::info!(
                "SftpServer ReadDir called for OTA SFTP server on handle: {:?}",
                _opaque_dir_handle
            );
            Err(sunset_sftp::protocol::StatusCode::SSH_FX_EOF)
        }
    }

    fn realpath(&mut self, dir: &str) -> sunset_sftp::server::SftpOpResult<NameEntry<'_>> {
        log::info!("SftpServer RealPath: dir = {:?}", dir);
        Ok(NameEntry {
            filename: Filename::from("/"),
            _longname: Filename::from("/"),
            attrs: sunset_sftp::protocol::Attrs::default(),
        })
    }

    fn stats(
        &mut self,
        follow_links: bool,
        file_path: &str,
    ) -> sunset_sftp::server::SftpOpResult<sunset_sftp::protocol::Attrs> {
        log::error!(
            "SftpServer Stats operation not defined: follow_link = {:?}, \
            file_path = {:?}",
            follow_links,
            file_path
        );
        Err(sunset_sftp::protocol::StatusCode::SSH_FX_OP_UNSUPPORTED)
    }
}

/// UpdateProcessorState for OTA update processing
///
/// This enum defines the various states of the OTA update processing state machine and will control the flow of the update process.
#[derive(Debug)]
enum UpdateProcessorState {
    /// ReadingParameters state, OTA has started and the processor is obtaining metadata values until the firmware blob is reached
    ReadingParameters {
        ltv_holder: [u8; tlv::MAX_TLV_SIZE as usize],
        current_len: usize,
    },
    /// Downloading state, receiving firmware data, computing hash on the fly and writing to flash
    Downloading { received_size: u32 },
    /// In this state, the processor verifies the downloaded firmware image
    Verifying,
    /// Like idle, but after successful verification, ready to reboot and apply the update
    Finished,
    /// Error state, an error occurred during the OTA process
    Error(OtaError),
}

impl Default for UpdateProcessorState {
    fn default() -> Self {
        UpdateProcessorState::ReadingParameters {
            ltv_holder: [0; tlv::MAX_TLV_SIZE as usize],
            current_len: 0,
        }
    }
}

/// # UpdateProcessor for handling OTA update processing
///
/// This struct manages the state and processing of OTA updates received via SFTP. It will handle reading metadata, writing data, verifying, and applying updates.
///
/// It uses an internal state machine defined by [[UpdateProcessorState]] to track the progress of the update process.
///
/// It will also handle incoming data chunks and process them accordingly.
struct UpdateProcessor {
    state: UpdateProcessorState,
    /// Hasher computing the checksum of the downloaded firmware on the fly
    hasher: sha2::Sha256,
    header: Header,
}

impl UpdateProcessor {
    pub fn new() -> Self {
        Self {
            state: UpdateProcessorState::default(),
            hasher: sha2::Sha256::new(),
            header: Header {
                ota_type: None,
                firmware_blob_size: None,
                sha256_checksum: None,
            },
        }
    }

    /// Main processing function for handling incoming data chunks
    ///
    /// It processes data based on the current state of the update processor [[UpdateProcessorState]]. To first, read most metadata parameters, after that, write the data to the appropriate location. as it is received.
    ///
    /// It will try to consume as much data as possible from the provided buffer and return the number of bytes used.
    pub fn process_data_chunk(&mut self, _offset: u64, _data: &[u8]) -> Result<usize, OtaError> {
        let mut used_bytes = 0;
        log::debug!(
            "UpdateProcessor: Processing data chunk at offset {}, length {} in state {:?}",
            _offset,
            _data.len(),
            self.state
        );
        match self.state {
            UpdateProcessorState::ReadingParameters {
                mut ltv_holder,
                mut current_len,
            } => {
                // TODO: Implement LTV parsing logic here
                // Check if we have data in the ltv_holder buffer. If so add enough data to complete the LTV entry
                // Otherwise, try processing new LTV entries in place from the data buffer

                // Unknown LTV must be skipped gracefully

                // If LTV entry is the firmware blob, transition to Downloading state
                // only if we have the necessary parameters (ota_type, total_size, sha256_checksum)
                // ota_type must be OTA_FIRMWARE_BLOB_TYPE
                if self.header.ota_type != Some(tlv::OTA_TYPE_SSH_STAMP)
                    || self.header.firmware_blob_size.is_none()
                    || self.header.sha256_checksum.is_none()
                {
                    log::error!(
                        "UpdateProcessor: Missing required OTA parameters: ota_type = {:?}, total_size = {:?}, sha256_checksum = {:?}",
                        self.header.ota_type,
                        self.header.firmware_blob_size,
                        self.header.sha256_checksum
                    );
                    return Err(OtaError::IllegalOperation);
                }
                // total_size must be > 0 and TODO: smaller than the ota partition size
                // sha256_checksum must be Some

                self.state = UpdateProcessorState::Downloading { received_size: 0 };
                log::debug!("UpdateProcessor: Transitioning to downloading state");
            }
            // Add other states and their processing logic here
            _ => {
                log::warn!(
                    "UpdateProcessor: Received data in unexpected state: {:?}",
                    self.state
                );
            }
        }
        Ok(used_bytes)
    }

    // Add other parameters, such as verify, apply, check signature, etc.
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
/// OtaError for OTA update processing errors
enum OtaError {
    /// An operation was illegal in the current state
    IllegalOperation,
    /// Error writing data to flash memory
    WriteError,
    /// Verification of the downloaded firmware failed
    VerificationFailed,
}

/// Module to define the tlv types for OTA metadata
///
/// Unsophisticated implementation of the ssh-stamp OTA TLV types using sshwire traits.
///
/// If you are looking into improving this, consider looking into [proto.rs](https://github.com/mkj/sunset/blob/8e5d20916cf7b29111b90e4d3b7bb7827c9be8e5/sftp/src/proto.rs)
/// for an example on how to automate the generation of protocols with macros
pub mod tlv {
    use log::warn;
    use sunset::sshwire::{SSHDecode, SSHEncode, SSHSource};

    pub const OTA_TYPE: u8 = 0;
    pub const FIRMWARE_BLOB: u8 = 1;
    // pub const FIRMWARE_BLOB_LEN: usize = 4; // u32 length allowing blobs up to 4GB
    pub const SHA256_CHECKSUM: u8 = 2;

    // TODO: We could provide a new type for better debuging information
    pub const OTA_TYPE_SSH_STAMP: u32 = 0x73736873; // 'sshs' big endian in ASCII

    pub const CHECKSUM_LEN: u32 = 32;
    /// Maximum size for LTV (Length-Type-Value) entries in OTA metadata. Used during the reading of OTA parameters.
    pub const MAX_TLV_SIZE: u32 = 1 + 8 + CHECKSUM_LEN; // type + length + value

    /// Encodes the length and value of a sized values
    fn enc_len_val<SE>(
        value: &SE,
        s: &mut dyn sunset::sshwire::SSHSink,
    ) -> sunset::sshwire::WireResult<()>
    where
        SE: Sized + SSHEncode,
    {
        (core::mem::size_of::<SE>() as u32).enc(s)?;
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
        let val_len = u32::dec(s)?;
        if val_len != (core::mem::size_of::<SE>() as u32) {
            return Err(sunset::sshwire::WireError::PacketWrong);
        }
        Ok(())
    }

    /// OTA_TLV enum for OTA metadata LTV entries
    /// This TLV does not capture length as it will be captured during parsing
    /// Parsing will be done using sshwire types
    #[derive(Debug)]
    #[repr(u8)]
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
                Tlv::FirmwareBlob { size } => {
                    FIRMWARE_BLOB.enc(s)?;
                    enc_len_val(size, s)
                }
                Tlv::Sha256Checksum { checksum } => {
                    SHA256_CHECKSUM.enc(s)?;
                    enc_len_val(checksum, s)
                }
                Tlv::OtaType { ota_type } => {
                    OTA_TYPE.enc(s)?;
                    enc_len_val(ota_type, s)
                }
            }
        }
    }

    impl<'de> SSHDecode<'de> for Tlv {
        fn dec<S>(s: &mut S) -> sunset::sshwire::WireResult<Self>
        where
            S: sunset::sshwire::SSHSource<'de>,
        {
            u8::dec(s).and_then(|tlv_type| match tlv_type {
                FIRMWARE_BLOB => {
                    dec_check_val_len::<S, u32>(s)?;
                    Ok(Tlv::FirmwareBlob { size: u32::dec(s)? })
                }
                SHA256_CHECKSUM => {
                    if u32::dec(s)? != CHECKSUM_LEN {
                        return Err(sunset::sshwire::WireError::PacketWrong);
                    }
                    let mut checksum = [0u8; 32];
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
                    let len = u32::dec(s)?;
                    s.take(len as usize)?; // Skip unknown TLV value
                    Err(sunset::sshwire::WireError::UnknownPacket { number: tlv_type })
                }
            })
        }
    }

    /// An implementation of SSHSource based on [[sunset::sshwire::DecodeBytes]]
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
}

/// Header struct for OTA file header processing
///
/// This struct holds the metadata that will be used to validate the OTA file prior to applying the update.
///
/// The fields serialisation and deserialization
#[derive(Debug)]
pub struct Header {
    // Not part of the header data
    // hasher: sha2::Sha256,
    /// Type of OTA update being processed. Used for screening incorrect ota blobs quickly
    ota_type: Option<u32>,
    /// Total size of the firmware being downloaded, if known
    firmware_blob_size: Option<u32>,
    /// Expected sha256 checksum of the firmware, if provided
    pub sha256_checksum: Option<[u8; 32]>,
}

impl Header {
    /// Creates a new OTA header with the provided parameters
    ///
    /// Used during packing of OTA files. Therefore, not needed in the embedded side.
    #[cfg(not(target_os = "none"))]
    pub fn new(ota_type: u32, sha256_checksum: &[u8], firmware_blob_size: u32) -> Self {
        // TODO: Check that the sha256_checksum length is correct: 32 bytes
        let mut checksum_array = [0u8; 32];
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
    pub fn deserialize(buf: &[u8]) -> Result<(Self, usize), sunset::sshwire::WireError> {
        let buf_len = buf.len();
        let mut source = tlv::TlvsSource::new(buf);
        let mut ota_type = None;
        let mut firmware_blob_size = None;
        let mut sha256_checksum = None;

        while source.used() < buf_len {
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
                            check_ota_is_first_tlv(ota_type)?;
                            sha256_checksum = Some(checksum);
                        }
                        tlv::Tlv::FirmwareBlob { size } => {
                            check_ota_is_first_tlv(ota_type)?;
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

#[cfg(test)]
mod ota_tlv_tests {

    use crate::Header;
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
                ota_type: OTA_TYPE_SSH_STAMP,
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
            ota_type: OTA_TYPE_SSH_STAMP,
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
            Header::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

        assert_eq!(header.ota_type, Some(OTA_TYPE_SSH_STAMP));
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
            ota_type: OTA_TYPE_SSH_STAMP,
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
            Header::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

        assert_eq!(header.ota_type, Some(OTA_TYPE_SSH_STAMP));
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
            ota_type: OTA_TYPE_SSH_STAMP,
        };
        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_type)
            .expect("Failed to write OTA Type TLV");

        let firmware_blob_tlv = Tlv::FirmwareBlob { size: 2048 };
        offset += sshwire::write_ssh(&mut buffer[offset..], &firmware_blob_tlv)
            .expect("Failed to write Firmware Blob TLV");

        assert!(Header::deserialize(&buffer[..offset]).is_err());
    }

    #[test]
    fn deserializing_header_missing_firmware_blob() {
        let mut buffer = [0u8; 512];
        let mut offset = 0;

        let ota_type_tlv = Tlv::OtaType {
            ota_type: OTA_TYPE_SSH_STAMP,
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
            Header::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

        assert_eq!(header.ota_type, Some(OTA_TYPE_SSH_STAMP));
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
            ota_type: OTA_TYPE_SSH_STAMP,
        };
        offset += sshwire::write_ssh(&mut buffer[offset..], &ota_type_tlv)
            .expect("Failed to write OTA Type TLV");

        // Manually generating a valid unknown type
        let unknown_type: u8 = 99;
        let unknown_type_len = 4u32;
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
            Header::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

        assert_eq!(header.ota_type, Some(OTA_TYPE_SSH_STAMP));
        assert_eq!(header.firmware_blob_size, Some(2048));
        assert_eq!(header.sha256_checksum, None);
    }

    // TODO: Test more error cases, such as incomplete TLVs
}
