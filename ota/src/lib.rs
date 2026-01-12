#![cfg_attr(not(test), no_std)]
// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later
use sunset::sshwire::{BinString, SSHDecode, SSHSource, WireError};
use sunset_async::ChanInOut;
use sunset_sftp::{
    SftpHandler,
    handles::OpaqueFileHandle,
    protocol::{FileHandle, Filename, NameEntry, PFlags},
    server::SftpServer,
};

use core::hash::Hasher;

use log::{debug, error, info, warn};
use rustc_hash::FxHasher;
use sha2::{Digest, Sha256};

pub async fn run_ota_server(stdio: ChanInOut<'_>) -> Result<(), sunset::Error> {
    // Placeholder for OTA server logic
    // This function would handle the SFTP session and perform OTA updates
    warn!("WIP SFTP not implemented");
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
            warn!("sftp server loop finished with an error: {}", &e);
            Err(e)
        }
    }
}

/// This length is chosen to keep the file handle small
/// while still providing a reasonable level of uniqueness.
/// We are not expecting more than one OTA operation at a time.
const OPAQUE_HASH_LEN: usize = 4;
/// OtaOpaqueFileHandle for OTA SFTP server
///
/// Minimal implementation of an opaque file handle with a tiny hash
#[derive(Hash, Debug, Eq, PartialEq, Clone)]
struct OtaOpaqueFileHandle {
    // Define fields as needed for OTA file handle
    tiny_hash: [u8; OPAQUE_HASH_LEN],
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

        let mut tiny_hash = [0u8; OPAQUE_HASH_LEN];
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
            info!(
                "SftpServer Open operation: path = {:?}, write_permission = {:?}, handle = {:?}",
                path, self.write_permission, &handle
            );
            return Ok(handle);
        } else {
            error!(
                "SftpServer Open operation failed: already writing OTA, path = {:?}, attrs = {:?}",
                path, mode
            );
            return Err(sunset_sftp::protocol::StatusCode::SSH_FX_PERMISSION_DENIED);
        }
    }

    fn close(&mut self, handle: &T) -> sunset_sftp::server::SftpOpResult<()> {
        // TODO: At this point I need to reset the target if all is ok or reset the processor if not so we are
        // either loading a new firmware or ready to receive a correct one.

        info!("Close called for handle {:?}", handle);
        if let Some(current_handle) = &self.file_handle {
            if current_handle == handle {
                info!(
                    "SftpServer Close operation for OTA completed: handle = {:?}",
                    handle
                );
                self.file_handle = None;
                self.write_permission = false;

                // Good place to finalize the OTA update process
                return Ok(());
            } else {
                warn!(
                    "SftpServer Close operation failed: handle mismatch = {:?}",
                    handle
                );
                return Err(sunset_sftp::protocol::StatusCode::SSH_FX_FAILURE);
            }
        } else {
            warn!(
                "SftpServer Close operation granted on untracked handle: {:?}",
                handle
            );
            return Ok(()); // TODO: Handle close properly. You will need this for the OTA server
        }
    }

    // We are not interested on download operations for OTA. Only upload (write)
    fn read<const N: usize>(
        &mut self,
        opaque_file_handle: &T,
        offset: u64,
        len: u32,
        _reply: &mut sunset_sftp::server::ReadReply<'_, N>,
    ) -> impl core::future::Future<Output = sunset_sftp::error::SftpResult<()>> {
        async move {
            error!(
                "SftpServer Read operation not defined: handle = {:?}, offset = {:?}, len = {:?}",
                opaque_file_handle, offset, len
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
                    warn!(
                        "SftpServer Write operation denied: no write permission for handle = {:?}",
                        opaque_file_handle
                    );
                    return Err(sunset_sftp::protocol::StatusCode::SSH_FX_PERMISSION_DENIED);
                }
                info!(
                    "SftpServer Write operation for OTA: handle = {:?}, offset = {:?}, buf_len = {:?}",
                    opaque_file_handle,
                    offset,
                    buf.len()
                );

                self.processor
                    .process_data_chunk(offset, buf)
                    .map_err(|e| {
                        error!(
                            "SftpServer Write operation failed during OTA processing: {:?}",
                            e
                        );
                        sunset_sftp::protocol::StatusCode::SSH_FX_FAILURE
                    })?;
                return Ok(());
            }
        }

        warn!(
            "SftpServer Write operation failed: handle mismatch = {:?}",
            opaque_file_handle
        );
        return Err(sunset_sftp::protocol::StatusCode::SSH_FX_FAILURE);
    }

    fn opendir(&mut self, dir: &str) -> sunset_sftp::server::SftpOpResult<T> {
        let handle = T::new(dir);
        info!(
            "SftpServer OpenDir: dir = {:?}. Returning {:?}",
            dir, &handle
        );
        Ok(handle) // TODO: Store handle and use salt
    }

    // For OTA, we do not expect any directory listing
    fn readdir<const N: usize>(
        &mut self,
        _opaque_dir_handle: &T,
        _reply: &mut sunset_sftp::server::DirReply<'_, N>,
    ) -> impl core::future::Future<Output = sunset_sftp::server::SftpOpResult<()>> {
        async move {
            info!(
                "SftpServer ReadDir called for OTA SFTP server on handle: {:?}",
                _opaque_dir_handle
            );
            Err(sunset_sftp::protocol::StatusCode::SSH_FX_EOF)
        }
    }

    // For OTA, realpath will always return root
    fn realpath(&mut self, dir: &str) -> sunset_sftp::server::SftpOpResult<NameEntry<'_>> {
        info!("SftpServer RealPath: dir = {:?}", dir);
        Ok(NameEntry {
            filename: Filename::from("/"),
            _longname: Filename::from("/"),
            attrs: sunset_sftp::protocol::Attrs::default(),
        })
    }

    // For OTA, we do not expect stat operations
    fn stats(
        &mut self,
        follow_links: bool,
        file_path: &str,
    ) -> sunset_sftp::server::SftpOpResult<sunset_sftp::protocol::Attrs> {
        error!(
            "SftpServer Stats operation not defined: follow_link = {:?}, \
            file_path = {:?}",
            follow_links, file_path
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
        tlv_holder: [u8; tlv::MAX_TLV_SIZE as usize],
        current_len: usize,
    },
    /// Downloading state, receiving firmware data, computing hash on the fly and writing to flash
    Downloading { total_received_size: u32 },
    /// Like idle, but after successful verification, ready to reboot and apply the update
    Finished,
    /// Error state, an error occurred during the OTA process
    Error(OtaError),
}

impl Default for UpdateProcessorState {
    fn default() -> Self {
        UpdateProcessorState::ReadingParameters {
            tlv_holder: [0; tlv::MAX_TLV_SIZE as usize],
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
    hasher: Sha256,
    header: Header,
}

impl UpdateProcessor {
    pub fn new() -> Self {
        Self {
            state: UpdateProcessorState::default(),
            hasher: Sha256::new(),
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
    pub fn process_data_chunk(&mut self, _offset: u64, data: &[u8]) -> Result<(), OtaError> {
        debug!(
            "UpdateProcessor: Processing data chunk at offset {}, length {} in state {:?}",
            _offset,
            data.len(),
            self.state
        );
        let mut source = tlv::TlvsSource::new(&data);
        while source.remaining() > 0 {
            debug!("processor state : {:?}", self.state);

            match self.state {
                UpdateProcessorState::ReadingParameters {
                    mut tlv_holder,
                    mut current_len,
                } => {
                    match source.try_taking_bytes_for_tlv(&mut tlv_holder, &mut current_len) {
                        Err(WireError::RanOut) => {
                            // Not enough data to complete TLV, wait for more data
                            self.state = UpdateProcessorState::ReadingParameters {
                                tlv_holder,
                                current_len,
                            };
                            return Ok(());
                        }
                        Err(e) => {
                            error!("Error processing TLV: {:?}", e);
                            return Err(OtaError::InternalError);
                        }
                        Ok(_) => {
                            // Successfully read a TLV, continue processing
                        }
                    };

                    // At this point there should be a complete TLV to be decoded
                    info!(
                        "Decoding TLV from tlv_holder: {:?},  current_len: {}",
                        &tlv_holder, &current_len
                    );
                    let mut singular_source = tlv::TlvsSource::new(&tlv_holder[..current_len]);

                    match tlv::Tlv::dec(&mut singular_source) {
                        Ok(tlv) => match tlv {
                            tlv::Tlv::OtaType { ota_type } => {
                                // TODO: If the received ota_type does not match tlv::OTA_TYPE_VALUE_SSH_STAMP go to error state.
                                if ota_type != tlv::OTA_TYPE_VALUE_SSH_STAMP {
                                    self.state =
                                        UpdateProcessorState::Error(OtaError::IllegalOperation);
                                    error!(
                                        "UpdateProcessor: Unsupported OTA Type received: {:?}",
                                        ota_type
                                    );
                                    return Ok(());
                                }
                                info!("Received Ota type: {:?}", ota_type);
                                self.header.ota_type = Some(ota_type);
                                self.state = UpdateProcessorState::ReadingParameters {
                                    tlv_holder: [0; tlv::MAX_TLV_SIZE as usize],
                                    current_len: 0,
                                };
                            }

                            tlv::Tlv::Sha256Checksum { checksum } => {
                                info!("Received Checksum: {:?}", &checksum);
                                if self.header.ota_type.is_none() {
                                    error!(
                                        "UpdateProcessor: Received SHA256 Checksum TLV before OTA Type TLV"
                                    );
                                    self.state =
                                        UpdateProcessorState::Error(OtaError::IllegalOperation);
                                    return Ok(());
                                }
                                self.header.sha256_checksum = Some(checksum);
                                self.state = UpdateProcessorState::ReadingParameters {
                                    tlv_holder: [0; tlv::MAX_TLV_SIZE as usize],
                                    current_len: 0,
                                };
                            }
                            tlv::Tlv::FirmwareBlob { size } => {
                                info!("Received FirmwareBlob size: {:?}", size);
                                if self.header.ota_type.is_none() {
                                    error!(
                                        "UpdateProcessor: Received SHA256 Checksum TLV before OTA Type TLV"
                                    );
                                    self.state =
                                        UpdateProcessorState::Error(OtaError::IllegalOperation);
                                    return Ok(());
                                }

                                if self.header.sha256_checksum.is_none() {
                                    error!(
                                        "UpdateProcessor: Received FirmwareBlob TLV before SHA256 Checksum TLV"
                                    );
                                    self.state =
                                        UpdateProcessorState::Error(OtaError::IllegalOperation);
                                    return Ok(());
                                }

                                self.header.firmware_blob_size = Some(size);
                                // Transition to Downloading state will be done after this match
                                self.state = UpdateProcessorState::Downloading {
                                    total_received_size: 0,
                                };
                                info!("Transitioning to Downloading state");
                            }
                        },
                        Err(WireError::UnknownPacket { number }) => {
                            warn!(
                                "UpdateProcessor: Unknown TLV type encountered: {}. Skipping it",
                                number
                            );
                            if self.header.ota_type.is_none() {
                                error!(
                                    "UpdateProcessor: Received unknown TLV type before OTA Type TLV"
                                );
                                self.state =
                                    UpdateProcessorState::Error(OtaError::IllegalOperation);
                                return Ok(());
                            }
                            // Skip this TLV and continue
                            self.state = UpdateProcessorState::ReadingParameters {
                                tlv_holder: [0; tlv::MAX_TLV_SIZE as usize],
                                current_len: 0,
                            }
                        }
                        Err(WireError::RanOut) => {
                            // Keep current data and wait for more
                            self.state = UpdateProcessorState::ReadingParameters {
                                tlv_holder,
                                current_len,
                            };
                            error!("UpdateProcessor: RanOut should not be happening");
                            return Err(OtaError::MoreDataRequired);
                        }
                        Err(e) => {
                            error!("Handle {:?} appropriately", e);
                            return Err(OtaError::InternalError);
                        }
                    }
                }
                UpdateProcessorState::Downloading {
                    mut total_received_size,
                } => {
                    let total_blob_size = match self.header.firmware_blob_size {
                        Some(size) => size,
                        None => {
                            error!(
                                "UpdateProcessor: Firmware blob size not set before downloading"
                            );
                            return Err(OtaError::IllegalOperation);
                        }
                    };
                    // Once the totallity of the blob has been received the FSM must move to the Finished or Error States
                    if total_received_size >= total_blob_size {
                        error!(
                            "UpdateProcessor: Received more data than expected: received_size = {}, total_blob_size = {}",
                            total_received_size, total_blob_size
                        );
                        return Err(OtaError::IllegalOperation);
                    }

                    let to_take = data
                        .len()
                        .min((total_blob_size - total_received_size) as usize);

                    let data_chunk = source.take(to_take).map_err(|e| {
                        error!(
                            "UpdateProcessor: Error taking data chunk of size {}: {:?}",
                            to_take, e
                        );
                        OtaError::InternalError
                    })?;

                    // Update hasher with the new data chunk
                    self.hasher.update(data_chunk);

                    // TODO: Here you would write data_chunk to flash memory
                    info!(
                        "Writing {} bytes to flash at offset {}",
                        data_chunk.len(),
                        total_received_size
                    );

                    total_received_size += to_take as u32;

                    if total_received_size >= total_blob_size {
                        let Some(original_hash) = self.header.sha256_checksum else {
                            error!(
                                "UpdateProcessor: No original checksum to verify against after download"
                            );
                            return Err(OtaError::IllegalOperation);
                        };

                        // if *new_hash != original_hash {
                        if original_hash
                            != *self
                                .hasher
                                .clone()
                                .finalize()
                                .as_array()
                                .ok_or(OtaError::VerificationFailed)?
                        {
                            error!(
                                "UpdateProcessor: Checksum mismatch after download! Expected: {:x?}`",
                                original_hash
                            );
                            self.state = UpdateProcessorState::Error(OtaError::VerificationFailed);
                            return Ok(());
                        } else {
                            info!("UpdateProcessor: Checksum verified successfully");
                        }

                        info!("All firmware data received, transitioning to Finished state");
                        self.state = UpdateProcessorState::Finished;
                    } else {
                        self.state = UpdateProcessorState::Downloading {
                            total_received_size,
                        };
                    }
                }
                UpdateProcessorState::Finished => {
                    // Will ignore the data. It will be consumed and the file will be closed eventually
                    warn!(
                        "UpdateProcessor: Received data in Finished state, ignoring additional data"
                    );
                    return Ok(());
                }
                UpdateProcessorState::Error(ota_error) => {
                    // Will ignore the data. It will be consumed and the file will be closed eventually
                    warn!(
                        "UpdateProcessor: Received data in Error state: {:?}, ignoring additional data",
                        ota_error
                    );
                    return Ok(());
                }
            };
        }
        Ok(())
    }

    // Add other parameters, such as verify, apply, check signature, etc.
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
/// OtaError for OTA update processing errors
enum OtaError {
    /// Needs more data to proceed
    MoreDataRequired,
    /// Internal error
    InternalError,
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
                tlv_holder[*current_len..*current_len + to_read]
                    .copy_from_slice(needed_type_len_bytes);
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
    pub sha256_checksum: Option<[u8; tlv::CHECKSUM_LEN as usize]>,
}

impl Header {
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
            Header::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

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
            Header::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

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

        assert!(Header::deserialize(&buffer[..offset]).is_err());
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
            Header::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

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
            Header::deserialize(&buffer[..offset]).expect("Failed to deserialize header");

        assert_eq!(header.ota_type, Some(OTA_TYPE_VALUE_SSH_STAMP));
        assert_eq!(header.firmware_blob_size, Some(2048));
        assert_eq!(header.sha256_checksum, None);
    }

    // TODO: Test more error cases, such as incomplete TLVs
}
