// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! UART implementation for the RP2350.
//!
//! Provides [`BufferedUart`] — a software-buffered, async, full-duplex UART
//! satisfying [`ssh_stamp::serial::BufferedSerial`]. Same shape as the ESP32
//! port: a task owns the hardware and pumps two pipes, so the SSH bridge can
//! poll TX and RX concurrently through `&self`.

use core::future::Future;

use embassy_rp::uart::{BufferedUart as RpBufferedUart, BufferedUartRx, BufferedUartTx};
use embassy_sync::pipe::TryWriteError;
use embassy_sync::signal::Signal;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use embedded_io_async::{Read, Write};
use portable_atomic::{AtomicUsize, Ordering};
use ssh_stamp::serial::BufferedSerial;
use static_cell::StaticCell;

const INWARD_BUF_SZ: usize = 512;
const OUTWARD_BUF_SZ: usize = 256;
const UART_BUF_SZ: usize = 64;

/// Bidirectional pipe buffer for UART communications.
pub struct BufferedUart {
    outward: Pipe<CriticalSectionRawMutex, OUTWARD_BUF_SZ>,
    inward: Pipe<CriticalSectionRawMutex, INWARD_BUF_SZ>,
    dropped_rx_bytes: AtomicUsize,
}

impl BufferedUart {
    #[must_use]
    pub fn new() -> Self {
        BufferedUart {
            outward: Pipe::new(),
            inward: Pipe::new(),
            dropped_rx_bytes: AtomicUsize::from(0),
        }
    }

    /// Transfer data between UART hardware and internal buffers.
    pub async fn run(&self, rx: &mut BufferedUartRx, tx: &mut BufferedUartTx) {
        let mut rx_buf = [0u8; UART_BUF_SZ];
        let mut tx_buf = [0u8; UART_BUF_SZ];

        loop {
            use embassy_futures::select::select;

            let rd_from = async {
                loop {
                    // `BufferedUartRx` returns as soon as anything is
                    // available, which is what a byte bridge wants; the
                    // DMA `Uart` would block until the buffer filled.
                    let Ok(n) = rx.read(&mut rx_buf).await else {
                        continue;
                    };

                    let mut rx_slice = &rx_buf[..n];

                    while !rx_slice.is_empty() {
                        rx_slice = match self.inward.try_write(rx_slice) {
                            Ok(w) => &rx_slice[w..],
                            Err(TryWriteError::Full) => {
                                let mut drop_buf = [0u8; UART_BUF_SZ];
                                let dropped = self
                                    .inward
                                    .try_read(&mut drop_buf[..rx_slice.len()])
                                    .unwrap_or(0);
                                let _ = self.dropped_rx_bytes.fetch_update(
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                    |d| Some(d.saturating_add(dropped)),
                                );
                                rx_slice
                            }
                        };
                    }
                }
            };

            let rd_to = async {
                loop {
                    let n = self.outward.read(&mut tx_buf).await;
                    let _ = tx.write_all(&tx_buf[..n]).await;
                }
            };

            select(rd_from, rd_to).await;
        }
    }

    pub async fn read(&self, buf: &mut [u8]) -> usize {
        self.inward.read(buf).await
    }

    pub async fn write(&self, buf: &[u8]) {
        self.outward.write_all(buf).await;
    }

    /// Number of bytes the RX side dropped since the last call. Resets the counter.
    pub fn check_dropped_bytes(&self) -> usize {
        self.dropped_rx_bytes.swap(0, Ordering::Relaxed)
    }
}

impl Default for BufferedUart {
    fn default() -> Self {
        Self::new()
    }
}

impl BufferedSerial for BufferedUart {
    fn read(&self, buf: &mut [u8]) -> impl Future<Output = usize> {
        BufferedUart::read(self, buf)
    }

    fn write(&self, buf: &[u8]) -> impl Future<Output = ()> {
        BufferedUart::write(self, buf)
    }

    fn check_dropped_bytes(&self) -> usize {
        BufferedUart::check_dropped_bytes(self)
    }
}

/// Static storage for the buffered UART singleton.
pub static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();

/// Signal raised by [`ssh_stamp::platform::PlatformServices::activate_uart`]
/// to release [`uart_task`] from its initial wait.
pub static UART_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();

/// Embassy task owning the hardware UART, pumping it through
/// [`BufferedUart::run`].
#[embassy_executor::task]
pub async fn uart_task(uart_buf: &'static BufferedUart, uart: RpBufferedUart) {
    UART_SIGNAL.wait().await;

    let (mut tx, mut rx) = uart.split();
    uart_buf.run(&mut rx, &mut tx).await;
}
