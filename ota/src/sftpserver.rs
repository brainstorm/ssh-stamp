// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::hash::Hasher;

use crate::{handler::UpdateProcessor, traits::OtaActions};

use sunset::sshwire::{BinString, WireError};
use sunset_async::ChanInOut;
use sunset_sftp::{
    SftpHandler,
    handles::{InitWithSeed, OpaqueFileHandle},
    protocol::{FileHandle, Filename, NameEntry, PFlags, StatusCode},
    server::{MAX_REQUEST_LEN, SftpServer},
};

use log::{debug, error, info, warn};
use rustc_hash::FxHasher;

/// Runs the OTA SFTP server
///
/// # Errors
/// Returns an error if the SFTP server loop encounters an error
pub async fn run_ota_server<W: OtaActions>(
    stdio: ChanInOut<'_>,
    ota_writer: W,
) -> Result<(), sunset::Error> {
    let mut buffer_in = [0u8; 512];
    let mut request_buffer = [0u8; MAX_REQUEST_LEN];

    let mut file_server = SftpOtaServer::new(ota_writer);

    match SftpHandler::<OtaOpaqueFileHandle, SftpOtaServer<OtaOpaqueFileHandle, W>, 512>::new(
        &mut file_server,
        &mut request_buffer,
    )
    .process_loop(stdio, &mut buffer_in)
    .await
    {
        Ok(()) => {
            debug!("sftp server loop finished gracefully");
            Ok(())
        }
        Err(e) => {
            warn!("sftp server loop finished with an error: {e:?}");
            Err(e.into())
        }
    }
}

/// This length is chosen to keep the file handle small
/// while still providing a reasonable level of uniqueness.
/// We are not expecting more than one OTA operation at a time.
const OPAQUE_HASH_LEN: usize = 4;

/// `OtaOpaqueFileHandle` for OTA SFTP server
///
/// Minimal implementation of an opaque file handle with a tiny hash
#[derive(Hash, Debug, Eq, PartialEq, Clone)]
struct OtaOpaqueFileHandle {
    // Define fields as needed for OTA file handle
    tiny_hash: [u8; OPAQUE_HASH_LEN],
}

impl OpaqueFileHandle for OtaOpaqueFileHandle {
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

impl InitWithSeed for OtaOpaqueFileHandle {
    type Err = WireError;

    fn init_with_seed(seed: &str) -> Result<Self, Self::Err> {
        let mut hasher = FxHasher::default();
        hasher.write(seed.as_bytes());
        let hash_bytes = u32::try_from(hasher.finish()).unwrap_or(0).to_be_bytes();
        Ok(OtaOpaqueFileHandle {
            tiny_hash: hash_bytes,
        })
    }
}

/// SFTP server implementation for OTA updates
///
/// This struct implements the `SftpServer` trait for handling OTA updates over SFTP
/// For now, all methods log an error and return unsupported operation as this is a placeholder
struct SftpOtaServer<T, W: OtaActions> {
    // Add fields as necessary for OTA server state
    file_handle: Option<T>,
    write_permission: bool,
    processor: UpdateProcessor<W>,
}

impl<T, W: OtaActions> SftpOtaServer<T, W> {
    pub fn new(ota_writer: W) -> Self {
        Self {
            // Initialize fields as necessary
            file_handle: None,
            write_permission: false,
            processor: UpdateProcessor::new(ota_writer),
        }
    }
}

impl<T: OpaqueFileHandle + InitWithSeed, W: OtaActions> SftpServer<'_, T> for SftpOtaServer<T, W> {
    async fn open(&'_ mut self, path: &str, mode: &PFlags) -> sunset_sftp::server::SftpOpResult<T> {
        if self.file_handle.is_none() {
            let num_mode = u32::from(mode);

            self.write_permission = num_mode & u32::from(&PFlags::SSH_FXF_WRITE) > 0
                || num_mode & u32::from(&PFlags::SSH_FXF_APPEND) > 0
                || num_mode & u32::from(&PFlags::SSH_FXF_CREAT) > 0;

            let handle = T::init_with_seed(path).map_err(|_| StatusCode::SSH_FX_FAILURE)?;
            self.file_handle = Some(handle.clone());
            info!(
                "SftpServer Open operation: path = {:?}, write_permission = {:?}, handle = {:?}",
                path, self.write_permission, &handle
            );
            Ok(handle)
        } else {
            error!(
                "SftpServer Open operation failed: already writing OTA, path = {path:?}, attrs = {mode:?}"
            );
            Err(StatusCode::SSH_FX_PERMISSION_DENIED)
        }
    }

    async fn close(&mut self, handle: &T) -> sunset_sftp::server::SftpOpResult<()> {
        // TODO: At this point I need to reset the target if all is ok or reset the processor if not so we are
        // either loading a new firmware or ready to receive a correct one.
        info!("Close called for handle {handle:?}");
        if let Some(current_handle) = &self.file_handle {
            if current_handle == handle {
                let ret_val = match self.processor.finalize().await {
                    Ok(()) => {
                        info!("OTA update finalized successfully.");
                        self.processor.reset_device();
                        Ok(())
                    }
                    Err(e) => {
                        error!("OTA update finalization failed: {e:?}");
                        Err(StatusCode::SSH_FX_FAILURE)
                    }
                };
                info!("SftpServer Close operation for OTA completed: handle = {handle:?}");
                self.file_handle = None;
                self.write_permission = false;

                ret_val
            } else {
                warn!("SftpServer Close operation failed: handle mismatch = {handle:?}");
                Err(StatusCode::SSH_FX_FAILURE)
            }
        } else {
            warn!("SftpServer Close operation granted on untracked handle: {handle:?}");
            Ok(())
        }
    }

    async fn read<const N: usize>(
        &mut self,
        opaque_file_handle: &T,
        offset: u64,
        len: u32,
        _reply: &mut sunset_sftp::server::ReadReply<'_, N>,
    ) -> sunset_sftp::error::SftpResult<()> {
        error!(
            "SftpServer Read operation not defined: handle = {opaque_file_handle:?}, offset = {offset:?}, len = {len:?}"
        );
        Err(sunset_sftp::error::SftpError::FileServerError(
            StatusCode::SSH_FX_OP_UNSUPPORTED,
        ))
    }

    async fn write(
        &mut self,
        opaque_file_handle: &T,
        offset: u64,
        buf: &[u8],
    ) -> sunset_sftp::server::SftpOpResult<()> {
        if let Some(current_handle) = &self.file_handle {
            if current_handle == opaque_file_handle {
                if !self.write_permission {
                    warn!(
                        "SftpServer Write operation denied: no write permission for handle = {opaque_file_handle:?}"
                    );
                    return Err(StatusCode::SSH_FX_PERMISSION_DENIED);
                }
                debug!(
                    "SftpServer Write operation for OTA: handle = {opaque_file_handle:?}, offset = {offset:?}, buf_len = {:?}",
                    buf.len()
                );

                if let Err(e) = self.processor.process_data(offset, buf).await {
                    match e {
                        crate::handler::OtaError::IllegalOperation => {
                            error!(
                                "SftpServer Write operation failed during OTA processing: Illegal Operation - {e:?}"
                            );
                            return Err(StatusCode::SSH_FX_PERMISSION_DENIED);
                        }
                        crate::handler::OtaError::UnknownTlvType => {
                            error!(
                                "SftpServer Write operation failed during OTA processing: Unknown TLV Type - {e:?}"
                            );
                            return Err(StatusCode::SSH_FX_OP_UNSUPPORTED);
                        }
                        _ => {
                            error!(
                                "SftpServer Write operation failed during OTA processing: {e:?}"
                            );
                            return Err(StatusCode::SSH_FX_FAILURE);
                        }
                    }
                }
                debug!(
                    "SftpServer Write operation for OTA processed successfully: handle = {opaque_file_handle:?}, offset = {offset:?}, buf_len = {:?}",
                    buf.len()
                );
                return Ok(());
            }
        }

        warn!("SftpServer Write operation failed: handle mismatch = {opaque_file_handle:?}");
        Err(StatusCode::SSH_FX_FAILURE)
    }

    async fn opendir(&mut self, dir: &str) -> sunset_sftp::server::SftpOpResult<T> {
        let handle = T::init_with_seed(dir).map_err(|_| StatusCode::SSH_FX_FAILURE)?;
        info!("SftpServer OpenDir: dir = {dir:?}. Returning {handle:?}",);
        Ok(handle)
    }

    async fn readdir<const N: usize>(
        &mut self,
        opaque_dir_handle: &T,
        _reply: &mut sunset_sftp::server::DirReply<'_, N>,
    ) -> sunset_sftp::server::SftpOpResult<()> {
        info!("SftpServer ReadDir called for OTA SFTP server on handle: {opaque_dir_handle:?}");
        Err(StatusCode::SSH_FX_EOF)
    }

    async fn realpath(&mut self, dir: &str) -> sunset_sftp::server::SftpOpResult<NameEntry<'_>> {
        info!("SftpServer RealPath: dir = {dir:?}");
        Ok(NameEntry {
            filename: Filename::from("/"),
            _longname: Filename::from("/"),
            attrs: sunset_sftp::protocol::Attrs::default(),
        })
    }

    async fn attrs(
        &mut self,
        follow_links: bool,
        file_path: &str,
    ) -> sunset_sftp::server::SftpOpResult<sunset_sftp::protocol::Attrs> {
        error!(
            "SftpServer Stats operation not defined: follow_link = {follow_links:?}, \
            file_path = {file_path:?}"
        );
        Err(StatusCode::SSH_FX_OP_UNSUPPORTED)
    }
}
