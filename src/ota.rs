// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use esp_bootloader_esp_idf::ota::Ota;
use sunset::sshwire::{BinString, WireError};
/// OTA update module over SFTP
use sunset_async::ChanInOut;
use sunset_sftp::{
    SftpHandler, handles::OpaqueFileHandle, protocol::FileHandle, server::SftpServer,
};

use core::hash::Hasher;

use esp_println::dbg;
use rustc_hash::FxHasher;

pub(crate) async fn run_ota_server(stdio: ChanInOut<'_>) -> Result<(), sunset::Error> {
    // Placeholder for OTA server logic
    // This function would handle the SFTP session and perform OTA updates
    dbg!("SFTP not implemented");
    let mut buffer_in = [0u8; 512];
    let mut request_buffer = [0u8; 512];

    match {
        let mut file_server = SftpOtaServer::new();

        SftpHandler::<OtaOpaqueFileHandle, SftpOtaServer, 512>::new(
            &mut file_server,
            &mut request_buffer,
        )
        .process_loop(stdio, &mut buffer_in)
        .await?;

        Ok::<_, sunset::Error>(())
    } {
        Ok(_) => {
            dbg!("sftp server loop finished gracefully");
            return Ok(());
        }
        Err(e) => {
            dbg!("sftp server loop finished with an error: {}", &e);
            return Err(e);
        }
    };
    Ok(())
}

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
struct SftpOtaServer;

impl SftpOtaServer {
    pub fn new() -> Self {
        SftpOtaServer
    }
}

impl<'a, T: OpaqueFileHandle> SftpServer<'a, T> for SftpOtaServer {
    fn open(
        &'_ mut self,
        path: &str,
        mode: &sunset_sftp::protocol::PFlags,
    ) -> sunset_sftp::server::SftpOpResult<T> {
        log::error!(
            "SftpServer Open operation not defined: path = {:?}, attrs = {:?}",
            path,
            mode
        );
        Err(sunset_sftp::protocol::StatusCode::SSH_FX_OP_UNSUPPORTED)
    }

    fn close(&mut self, handle: &T) -> sunset_sftp::server::SftpOpResult<()> {
        log::error!(
            "SftpServer Close operation not defined: handle = {:?}",
            handle
        );

        Err(sunset_sftp::protocol::StatusCode::SSH_FX_OP_UNSUPPORTED)
    }

    fn read<const N: usize>(
        &mut self,
        opaque_file_handle: &T,
        offset: u64,
        len: u32,
        reply: &mut sunset_sftp::server::ReadReply<'_, N>,
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
        log::error!(
            "SftpServer Write operation not defined: handle = {:?}, offset = {:?}, buf = {:?}",
            opaque_file_handle,
            offset,
            buf
        );
        Ok(())
    }

    fn opendir(&mut self, dir: &str) -> sunset_sftp::server::SftpOpResult<T> {
        log::error!("SftpServer OpenDir operation not defined: dir = {:?}", dir);
        Err(sunset_sftp::protocol::StatusCode::SSH_FX_OP_UNSUPPORTED)
    }

    fn readdir<const N: usize>(
        &mut self,
        opaque_dir_handle: &T,
        reply: &mut sunset_sftp::server::DirReply<'_, N>,
    ) -> impl core::future::Future<Output = sunset_sftp::server::SftpOpResult<()>> {
        async move {
            log::error!(
                "SftpServer ReadDir operation not defined: handle = {:?}",
                opaque_dir_handle
            );
            Err(sunset_sftp::protocol::StatusCode::SSH_FX_OP_UNSUPPORTED)
        }
    }

    fn realpath(
        &mut self,
        dir: &str,
    ) -> sunset_sftp::server::SftpOpResult<sunset_sftp::protocol::NameEntry<'_>> {
        log::error!("SftpServer RealPath operation not defined: dir = {:?}", dir);
        Err(sunset_sftp::protocol::StatusCode::SSH_FX_OP_UNSUPPORTED)
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

    // Implement required methods for the SftpServer trait
}
