// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! UART implementation for ESP32 family.
//!
//! Provides [`BufferedUart`] — a software-buffered, async, full-duplex UART
//! satisfying [`ssh_stamp::serial::BufferedSerial`]. The bridge can poll the
//! same UART from two futures (TX and RX) concurrently because both sides
//! take `&self`.

use core::future::Future;

use embassy_sync::pipe::TryWriteError;
use embassy_sync::signal::Signal;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use esp_hal::Async;
use esp_hal::gpio::AnyPin;
use esp_hal::peripherals::UART1;
use esp_hal::uart::{Config, RxConfig, Uart};
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
    ///
    /// This should be awaited from an Embassy task run in an `InterruptExecutor`
    /// for lower latency.
    pub async fn run(&self, uart: Uart<'_, Async>) {
        let (mut uart_rx, mut uart_tx) = uart.split();
        let mut rx_buf = [0u8; UART_BUF_SZ];
        let mut tx_buf = [0u8; UART_BUF_SZ];

        loop {
            use embassy_futures::select::select;

            let rd_from = async {
                loop {
                    let Ok(n) = uart_rx.read_async(&mut rx_buf).await else {
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
                                    .unwrap_or_default();
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
                    let _ = uart_tx.write_async(&tx_buf[..n]).await;
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

/// UART pins configuration.
pub struct EspUartPins<'a> {
    pub rx: AnyPin<'a>,
    pub tx: AnyPin<'a>,
}

/// Static storage for the buffered UART singleton.
pub static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();

/// Signal raised by [`ssh_stamp::platform::PlatformServices::activate_uart`]
/// to release [`uart_task`] from its initial wait.
pub static UART_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();

/// Embassy task that owns the hardware UART and pumps it through
/// [`BufferedUart::run`]. Spawn from a higher-priority `InterruptExecutor`
/// for lower latency.
#[embassy_executor::task]
pub async fn uart_task(
    uart_buf: &'static BufferedUart,
    uart1: UART1<'static>,
    pins: EspUartPins<'static>,
) {
    UART_SIGNAL.wait().await;

    let uart_config = Config::default().with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1),
    );

    let Ok(uart) = Uart::new(uart1, uart_config) else {
        return;
    };
    let uart = uart.with_rx(pins.rx).with_tx(pins.tx).into_async();

    uart_buf.run(uart).await;
}
