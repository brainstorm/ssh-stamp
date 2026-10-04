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
use log::{debug, info, warn};

/// Ctrl-D (EOT) as sent by a terminal.
const CTRL_D: u8 = 0x04;

/// What the bridge does with a Ctrl-D (0x04) from the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtrlD {
    /// Ends the session, like Ctrl-D at a shell prompt. Bytes before it are
    /// still forwarded.
    Exit,
    /// Forwards it to the target like any other byte.
    Forward,
}

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
/// the connection drops or, with [`CtrlD::Exit`], the client sends Ctrl-D.
/// # Errors
/// Returns an error if the SSH connection fails.
pub async fn serial_bridge<U: BufferedSerial>(
    chan_read: impl Read<Error = sunset::Error>,
    chan_write: impl Write<Error = sunset::Error>,
    uart: &U,
    ctrl_d: CtrlD,
) -> Result<(), sunset::Error> {
    debug!("Starting serial <--> SSH bridge");
    select(
        uart_to_ssh(uart, chan_write),
        ssh_to_uart(chan_read, uart, ctrl_d),
    )
    .await;
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
    ctrl_d: CtrlD,
) -> Result<(), sunset::Error> {
    let mut uart_tx_buf = [0u8; 64];
    loop {
        let n = chan_read.read(&mut uart_tx_buf).await?;
        if n == 0 {
            return Err(sunset::Error::ChannelEOF);
        }
        let data = &uart_tx_buf[..n];
        if ctrl_d == CtrlD::Exit
            && let Some(i) = data.iter().position(|&b| b == CTRL_D)
        {
            uart_buf.write(&data[..i]).await;
            info!("Ctrl-D from the client, ending the session");
            return Ok(());
        }
        uart_buf.write(data).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::future::{pending, ready};
    use std::sync::Mutex;
    use std::vec::Vec;

    /// Records what the bridge writes to the target.
    struct Uart(Mutex<Vec<u8>>);

    impl BufferedSerial for Uart {
        fn read(&self, _buf: &mut [u8]) -> impl Future<Output = usize> {
            pending()
        }

        fn write(&self, buf: &[u8]) -> impl Future<Output = ()> {
            self.0.lock().unwrap().extend_from_slice(buf);
            ready(())
        }

        fn check_dropped_bytes(&self) -> usize {
            0
        }
    }

    /// A client that sends `self.0` and then EOF.
    struct Client(&'static [u8]);

    impl embedded_io_async::ErrorType for Client {
        type Error = sunset::Error;
    }

    impl Read for Client {
        fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize, sunset::Error>> {
            let n = self.0.len().min(buf.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            ready(Ok(n))
        }
    }

    fn to_uart(input: &'static [u8], ctrl_d: CtrlD) -> (Result<(), sunset::Error>, Vec<u8>) {
        let uart = Uart(Mutex::new(Vec::new()));
        let result = embassy_futures::block_on(ssh_to_uart(Client(input), &uart, ctrl_d));
        (result, uart.0.into_inner().unwrap())
    }

    #[test]
    fn ctrl_d_ends_the_session_after_forwarding_what_came_before() {
        let (result, sent) = to_uart(b"ls\x04rest", CtrlD::Exit);
        assert!(result.is_ok());
        assert_eq!(sent, b"ls");
    }

    #[test]
    fn transparent_sessions_forward_ctrl_d() {
        let (result, sent) = to_uart(b"ls\x04rest", CtrlD::Forward);
        assert!(matches!(result, Err(sunset::Error::ChannelEOF)));
        assert_eq!(sent, b"ls\x04rest");
    }
}
