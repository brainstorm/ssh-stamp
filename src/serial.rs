// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use embassy_futures::select::select;
use embedded_io_async::{Read, Write};
use log::{debug, info, warn};

// Espressif specific crates
use crate::espressif::buffered_uart::BufferedUart;

/// Forwards an incoming SSH connection to/from the local UART, until
/// the connection drops
/// # Errors
/// Returns an error if the SSH connection fails
pub async fn serial_bridge(
    chan_read: impl Read<Error = sunset::Error>,
    chan_write: impl Write<Error = sunset::Error>,
    uart: &BufferedUart,
) -> Result<(), sunset::Error> {
    info!("Starting serial <--> SSH bridge");
    select(uart_to_ssh(uart, chan_write), ssh_to_uart(chan_read, uart)).await;
    debug!("Stopping serial <--> SSH bridge");
    Ok(())
}

async fn uart_to_ssh(
    uart_buf: &BufferedUart,
    mut chan_write: impl Write<Error = sunset::Error>,
) -> Result<(), sunset::Error> {
    let mut ssh_tx_buf = [0u8; 512];
    loop {
        let dropped = uart_buf.check_dropped_bytes();
        if dropped > 0 {
            // TODO: should this also go to the SSH client?
            warn!("UART RX dropped {dropped} bytes");
        }
        let n = uart_buf.read(&mut ssh_tx_buf).await;
        chan_write.write_all(&ssh_tx_buf[..n]).await?;
    }
}

async fn ssh_to_uart(
    mut chan_read: impl Read<Error = sunset::Error>,
    uart_buf: &BufferedUart,
) -> Result<(), sunset::Error> {
    let mut uart_tx_buf = [0u8; 64];
    loop {
        let n = chan_read.read(&mut uart_tx_buf).await?;
        if n == 0 {
            return Err(sunset::Error::ChannelEOF);
        }
        uart_buf.write(&uart_tx_buf[..n]).await;
    }
}
