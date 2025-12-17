// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

// OTA update module

use esp_println::dbg;
use sunset_async::ChanInOut;
use sunset_sftp::{DemoOpaqueFileHandle, DemoSftpServer, SftpHandler};

pub(crate) async fn run_ota_server(stdio: ChanInOut<'_>) -> Result<(), sunset::Error> {
    // Placeholder for OTA server logic
    // This function would handle the SFTP session and perform OTA updates
    dbg!("SFTP not implemented");
    let mut buffer_in = [0u8; 512];
    let mut request_buffer = [0u8; 512];

    match {
        let stdio = serv.stdio(ch).await?;
        let mut file_server = DemoSftpServer::new(
            "./demo/sftp/std/testing/out/".to_string(),
        );

        SftpHandler::<DemoOpaqueFileHandle, DemoSftpServer, 512>::new(
            &mut file_server,
            &mut request_buffer,
        )
        .process_loop(stdio, &mut buffer_in)
        .await?;

        Ok::<_, Error>(())
    } {
        Ok(_) => {
            warn!("sftp server loop finished gracefully");
            return Ok(());
        }
        Err(e) => {
            error!("sftp server loop finished with an error: {}", e);
            return Err(e);
        }
    };
    Ok(())
}