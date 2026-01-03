// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use sunset::sshwire::{BinString, SSHDecode, SSHEncode, SSHSource, WireError, WireResult};
use sunset_async::ChanInOut;
use sunset_sftp::{
    SftpHandler,
    handles::OpaqueFileHandle,
    protocol::{FileHandle, Filename, NameEntry, PFlags},
    server::SftpServer,
};

use core::hash::Hasher;
use esp_println::dbg;
use rustc_hash::FxHasher;
use sha2::Digest;

pub(crate) async fn run_ota_server(stdio: ChanInOut<'_>) -> Result<(), sunset::Error> {
    // Placeholder for OTA server logic
    // This function would handle the SFTP session and perform OTA updates
    dbg!("SFTP not implemented");
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
            dbg!("sftp server loop finished gracefully");
            Ok(())
        }
        Err(e) => {
            dbg!("sftp server loop finished with an error: {}", &e);
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

// TODO: Adjust this size as needed based on expected LTV sizes instead of a wild guess
/// Maximum size for LTV (Length-Type-Value) entries in OTA metadata. Used during the reading of OTA parameters.
const MAX_LTV_SIZE: usize = 32;

const OTA_FIRMWARE_BLOB_TYPE: u32 = 0x73736873; // 'sshs' big endian in ASCII

/// UpdateProcessorState for OTA update processing
///
/// This enum defines the various states of the OTA update processing state machine and will control the flow of the update process.
#[derive(Debug)]
enum UpdateProcessorState {
    /// ReadingParameters state, OTA has started and the processor is obtaining metadata values until the firmware blob is reached
    ReadingParameters {
        ltv_holder: [u8; MAX_LTV_SIZE],
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
            ltv_holder: [0; MAX_LTV_SIZE],
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
    ota_type: Option<u32>,
    /// Total size of the firmware being downloaded, if known
    total_size: Option<u64>,
    /// Expected sha256 checksum of the firmware, if provided
    sha256_checksum: Option<[u8; 32]>,
}

impl UpdateProcessor {
    pub fn new() -> Self {
        Self {
            state: UpdateProcessorState::default(),
            hasher: sha2::Sha256::new(),
            ota_type: None,
            total_size: None,
            sha256_checksum: None,
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
                if self.ota_type != Some(OTA_FIRMWARE_BLOB_TYPE)
                    || self.total_size.is_none()
                    || self.sha256_checksum.is_none()
                {
                    log::error!(
                        "UpdateProcessor: Missing required OTA parameters: ota_type = {:?}, total_size = {:?}, sha256_checksum = {:?}",
                        self.ota_type,
                        self.total_size,
                        self.sha256_checksum
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
pub mod ota_tlv {
    use sunset::sshwire::{SSHDecode, SSHEncode};

    pub const FIRMWARE_BLOB: u8 = 0;
    pub const TOTAL_SIZE: u8 = 1;
    pub const SHA256_CHECKSUM: u8 = 2;
    pub const OTA_TYPE: u8 = 3;
    /// OTA_TLV enum for OTA metadata LTV entries
    /// This TLV does not capture length as it will be captured during parsing
    /// Parsing will be done using sshwire types
    #[derive(Debug)]
    pub enum OtaTlv {
        /// What follows is the firmware blob
        FirmwareBlob {},
        /// The blob's total size in bytes
        TotalSize { size: u64 },
        /// Expected SHA256 checksum of the firmware blob
        Sha256Checksum { checksum: [u8; 32] },
        /// Type of OTA update. For SSH Stamp, this must be OTA_FIRMWARE_BLOB_TYPE
        OtaType { ota_type: u32 },
    }

    impl SSHEncode for OtaTlv {
        fn enc(&self, s: &mut dyn sunset::sshwire::SSHSink) -> sunset::sshwire::WireResult<()> {
            todo!()
        }
    }

    impl<'de> SSHDecode<'de> for OtaTlv {
        fn dec<S>(s: &mut S) -> sunset::sshwire::WireResult<Self>
        where
            S: sunset::sshwire::SSHSource<'de>,
        {
            todo!()
        }
    }
}
