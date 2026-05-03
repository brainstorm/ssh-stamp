// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::future::Future;

use embassy_futures::select::select;
use embedded_io_async::{Read, Write};
use log::{debug, warn};

/// Platform-agnostic buffered serial bridge.
///
/// The serial bridge is the inner loop that pumps bytes between the SSH
/// channel and the target UART. Every platform provides a concrete type
/// implementing this trait (ESP32: `ssh_stamp_esp32::BufferedUart`).
///
/// `read`/`write` take `&self` (not `&mut self`) because the bridge splits
/// each direction into its own future and runs them concurrently via
/// [`embassy_futures::select::select`]. Implementations back this with
/// internal pipes / interrupt-filled buffers.
pub trait BufferedSerial: Sync {
    /// Read as many bytes as are available, up to `buf.len()`. Returns the
    /// number of bytes read. Awaits until at least one byte is available.
    fn read(&self, buf: &mut [u8]) -> impl Future<Output = usize>;

    /// Queue bytes to be written. Completes once `buf` has been accepted
    /// by the internal buffer (may still be in flight on the wire).
    fn write(&self, buf: &[u8]) -> impl Future<Output = ()>;

    /// Return how many received bytes were dropped since the last call
    /// due to the internal buffer being full. Resets the counter.
    fn check_dropped_bytes(&self) -> usize;
}

/// Forwards an incoming SSH connection to/from the local UART, until
/// the connection drops.
/// # Errors
/// Returns an error if the SSH connection fails.
pub async fn serial_bridge<U: BufferedSerial>(
    chan_read: impl Read<Error = sunset::Error>,
    chan_write: impl Write<Error = sunset::Error>,
    uart: &U,
) -> Result<(), sunset::Error> {
    debug!("Starting serial <--> SSH bridge");
    select(uart_to_ssh(uart, chan_write), ssh_to_uart(chan_read, uart)).await;
    debug!("Stopping serial <--> SSH bridge");
    Ok(())
}

async fn uart_to_ssh<U: BufferedSerial>(
    uart_buf: &U,
    mut chan_write: impl Write<Error = sunset::Error>,
) -> Result<(), sunset::Error> {
    let mut ssh_tx_buf = [0u8; 512];
    loop {
        let dropped = uart_buf.check_dropped_bytes();
        if dropped > 0 {
            warn!("UART RX dropped {dropped} bytes");
        }
        let n = uart_buf.read(&mut ssh_tx_buf).await;
        chan_write.write_all(&ssh_tx_buf[..n]).await?;
    }
}

async fn ssh_to_uart<U: BufferedSerial>(
    mut chan_read: impl Read<Error = sunset::Error>,
    uart_buf: &U,
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
