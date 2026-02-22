// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use embassy_futures::select::select;
use embedded_io_async::{Read, Write};

// Espressif specific crates
use crate::espressif::buffered_uart::BufferedUart;
use esp_println::println;

/// Forwards an incoming SSH connection to/from the local UART, until
/// the connection drops
pub async fn serial_bridge(
    chanr: impl Read<Error = sunset::Error>,
    chanw: impl Write<Error = sunset::Error>,
    uart: &BufferedUart,
) -> Result<(), sunset::Error> {
    println!("Starting serial <--> SSH bridge");

    select(uart_to_ssh(uart, chanw), ssh_to_uart(chanr, uart)).await;
    println!("Stopping serial <--> SSH bridge");
    Ok(())
}

async fn uart_to_ssh(
    uart_buf: &BufferedUart,
    mut chanw: impl Write<Error = sunset::Error>,
) -> Result<(), sunset::Error> {
    let mut ssh_tx_buf = [0u8; 512];
    loop {
        let dropped = uart_buf.check_dropped_bytes();
        if dropped > 0 {
            // TODO: should this also go to the SSH client?
            println!("UART RX dropped {} bytes", dropped);
        }
        let n = uart_buf.read(&mut ssh_tx_buf).await;
        chanw.write_all(&ssh_tx_buf[..n]).await?;
    }
}

async fn ssh_to_uart(
    mut chanr: impl Read<Error = sunset::Error>,
    uart_buf: &BufferedUart,
) -> Result<(), sunset::Error> {
    let mut uart_tx_buf = [0u8; 64];
    loop {
        let n = chanr.read(&mut uart_tx_buf).await?;
        if n == 0 {
            return Err(sunset::Error::ChannelEOF);
        }
        uart_buf.write(&uart_tx_buf[..n]).await;
    }
}
