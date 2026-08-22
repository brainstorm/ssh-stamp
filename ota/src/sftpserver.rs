// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::future::{Future, ready};

use crate::handler::{OtaError, UpdateProcessor};
use ssh_stamp_hal::OtaActions;

use sunset_async::ChanInOut;
use sunset_sftp::{
    SftpServerHandler,
    protocol::{Filename, NameEntry, PFlags, StatusCode},
    server::{
        DirHandle, DirReadHeaderReply, DirReadReplyFinished, FileHandle, MAX_REQUEST_LEN,
        SftpServer,
    },
};

use log::{debug, error, info, warn};
use sunset_sftp::embedded_io_async::Write;

/// The single handle this server hands out.
///
/// sunset-sftp 0.2 lets the server pick its own `u32` handle values, and an
/// OTA session only ever has one file open at a time, so a constant is
/// enough — the previous hashed opaque handle bought nothing.
const OTA_FILE_HANDLE: FileHandle = FileHandle(1);

/// Handle returned for directory opens, kept distinct from the file one for
/// clarity even though the protocol allows reusing the value.
const OTA_DIR_HANDLE: DirHandle = DirHandle(1);

/// Runs the OTA SFTP server
///
/// # Errors
/// Returns an error if the SFTP server loop encounters an error
pub async fn run_ota_server<W: OtaActions>(
    stdio: ChanInOut<'_>,
    ota_writer: W,
) -> Result<(), sunset::Error> {
    let mut file_server = SftpOtaServer::new(ota_writer);
    let mut handler = SftpServerHandler::<MAX_REQUEST_LEN, MAX_REQUEST_LEN>::new();

    let (chan_in, chan_out) = stdio.split();

    match handler.run(&mut file_server, chan_in, chan_out).await {
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

/// SFTP server implementation for OTA updates
///
/// Accepts exactly one file at a time and streams it into the OTA
/// partition; every other operation is refused.
struct SftpOtaServer<W: OtaActions> {
    /// `Some` while a file is open for this session.
    open_handle: Option<FileHandle>,
    write_permission: bool,
    processor: UpdateProcessor<W>,
}

impl<W: OtaActions> SftpOtaServer<W> {
    pub fn new(ota_writer: W) -> Self {
        Self {
            open_handle: None,
            write_permission: false,
            processor: UpdateProcessor::new(ota_writer),
        }
    }
}

impl<W: OtaActions> SftpServer for SftpOtaServer<W> {
    fn open(
        &mut self,
        path: &str,
        mode: &PFlags,
    ) -> impl Future<Output = sunset_sftp::server::SftpOpResult<FileHandle>> {
        if self.open_handle.is_some() {
            error!(
                "SftpServer Open operation failed: already writing OTA, path = {path:?}, attrs = {mode:?}"
            );
            return ready(Err(StatusCode::SSH_FX_PERMISSION_DENIED));
        }

        let num_mode = u32::from(mode);
        self.write_permission = num_mode & u32::from(&PFlags::SSH_FXF_WRITE) > 0
            || num_mode & u32::from(&PFlags::SSH_FXF_APPEND) > 0
            || num_mode & u32::from(&PFlags::SSH_FXF_CREAT) > 0;

        self.open_handle = Some(OTA_FILE_HANDLE);
        info!(
            "SftpServer Open operation: path = {:?}, write_permission = {:?}, handle = {:?}",
            path, self.write_permission, OTA_FILE_HANDLE
        );
        ready(Ok(OTA_FILE_HANDLE))
    }

    async fn close(&mut self, handle: FileHandle) -> sunset_sftp::server::SftpOpResult<()> {
        info!("Close called for handle {handle:?}");
        let Some(current_handle) = self.open_handle else {
            warn!("SftpServer Close operation granted on untracked handle: {handle:?}");
            return Ok(());
        };
        if current_handle != handle {
            warn!("SftpServer Close operation failed: handle mismatch = {handle:?}");
            return Err(StatusCode::SSH_FX_FAILURE);
        }

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
        self.open_handle = None;
        self.write_permission = false;

        ret_val
    }

    async fn write(
        &mut self,
        handle: FileHandle,
        offset: u64,
        buf: &[u8],
    ) -> sunset_sftp::server::SftpOpResult<()> {
        if self.open_handle != Some(handle) {
            warn!("SftpServer Write operation failed: handle mismatch = {handle:?}");
            return Err(StatusCode::SSH_FX_FAILURE);
        }
        if !self.write_permission {
            warn!("SftpServer Write operation denied: no write permission for handle = {handle:?}");
            return Err(StatusCode::SSH_FX_PERMISSION_DENIED);
        }

        debug!(
            "SftpServer Write operation for OTA: handle = {handle:?}, offset = {offset:?}, buf_len = {:?}",
            buf.len()
        );

        if let Err(e) = self.processor.process_data(offset, buf).await {
            return Err(match e {
                OtaError::IllegalOperation => {
                    error!(
                        "SftpServer Write operation failed during OTA processing: Illegal Operation - {e:?}"
                    );
                    StatusCode::SSH_FX_PERMISSION_DENIED
                }
                OtaError::UnknownTlvType => {
                    error!(
                        "SftpServer Write operation failed during OTA processing: Unknown TLV Type - {e:?}"
                    );
                    StatusCode::SSH_FX_OP_UNSUPPORTED
                }
                _ => {
                    error!("SftpServer Write operation failed during OTA processing: {e:?}");
                    StatusCode::SSH_FX_FAILURE
                }
            });
        }

        debug!(
            "SftpServer Write operation for OTA processed successfully: handle = {handle:?}, offset = {offset:?}, buf_len = {:?}",
            buf.len()
        );
        Ok(())
    }

    fn opendir(
        &mut self,
        dir: &str,
    ) -> impl Future<Output = sunset_sftp::server::SftpOpResult<DirHandle>> {
        info!("SftpServer OpenDir: dir = {dir:?}. Returning {OTA_DIR_HANDLE:?}");
        ready(Ok(OTA_DIR_HANDLE))
    }

    fn readdir<W2: Write>(
        &mut self,
        handle: DirHandle,
        _reply: DirReadHeaderReply<'_, '_, W2>,
    ) -> impl Future<Output = sunset_sftp::server::SftpOpResult<DirReadReplyFinished>> {
        info!("SftpServer ReadDir called for OTA SFTP server on handle: {handle:?}");
        ready(Err(StatusCode::SSH_FX_EOF))
    }

    fn realpath(
        &mut self,
        dir: &str,
    ) -> impl Future<Output = sunset_sftp::server::SftpOpResult<NameEntry<'_>>> {
        info!("SftpServer RealPath: dir = {dir:?}");
        ready(Ok(NameEntry {
            filename: Filename::from("/"),
            _longname: Filename::from("/"),
            attrs: sunset_sftp::protocol::Attrs::default(),
        }))
    }
}
