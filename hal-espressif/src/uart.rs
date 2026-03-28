// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! UART implementation for ESP32 family
//!
//! Uses DMA-based buffered I/O for efficient serial communication.

use embassy_sync::pipe::TryWriteError;
use embassy_sync::signal::Signal;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use esp_hal::gpio::AnyPin;
use esp_hal::peripherals::UART1;
use esp_hal::uart::{Config, RxConfig, Uart};
use esp_hal::Async;
use hal::{HalError, UartConfig, UartHal};
use portable_atomic::{AtomicUsize, Ordering};
use static_cell::StaticCell;

const INWARD_BUF_SZ: usize = 512;
const OUTWARD_BUF_SZ: usize = 256;
const UART_BUF_SZ: usize = 64;

/// Bidirectional pipe buffer for UART communications
pub struct BufferedUart {
    outward: Pipe<CriticalSectionRawMutex, OUTWARD_BUF_SZ>,
    inward: Pipe<CriticalSectionRawMutex, INWARD_BUF_SZ>,
    dropped_rx_bytes: AtomicUsize,
    signal: Signal<CriticalSectionRawMutex, ()>,
}

impl BufferedUart {
    pub fn new() -> Self {
        BufferedUart {
            outward: Pipe::new(),
            inward: Pipe::new(),
            dropped_rx_bytes: AtomicUsize::from(0),
            signal: Signal::new(),
        }
    }

    /// Transfer data between UART hardware and internal buffers.
    ///
    /// This should be awaited from an Embassy task run in an InterruptExecutor
    /// for lower latency.
    pub async fn run(&self, uart: Uart<'_, Async>) {
        let (mut uart_rx, mut uart_tx) = uart.split();
        let mut uart_rx_buf = [0u8; UART_BUF_SZ];
        let mut uart_tx_buf = [0u8; UART_BUF_SZ];

        loop {
            use embassy_futures::select::select;

            let rd_from = async {
                loop {
                    let Ok(n) = uart_rx.read_async(&mut uart_rx_buf).await else {
                        continue;
                    };

                    let mut rx_slice = &uart_rx_buf[..n];

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
                    let n = self.outward.read(&mut uart_tx_buf).await;
                    let _ = uart_tx.write_async(&uart_tx_buf[..n]).await;
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

    /// Return the number of dropped bytes since last check and reset counter
    pub fn check_dropped_bytes(&self) -> usize {
        self.dropped_rx_bytes.swap(0, Ordering::Relaxed)
    }

    /// Signal that UART should start
    pub fn signal(&self) -> &Signal<CriticalSectionRawMutex, ()> {
        &self.signal
    }
}

impl Default for BufferedUart {
    fn default() -> Self {
        Self::new()
    }
}

/// UART pins configuration
pub struct EspUartPins<'a> {
    pub rx: AnyPin<'a>,
    pub tx: AnyPin<'a>,
}

/// ESP UART implementation
pub struct EspUart {
    buffered: &'static BufferedUart,
    configured: bool,
}

impl EspUart {
    /// Create a new ESP UART instance
    pub fn new(buffered: &'static BufferedUart) -> Self {
        Self {
            buffered,
            configured: false,
        }
    }

    /// Get the buffered UART for task spawning
    pub fn buffered(&self) -> &'static BufferedUart {
        self.buffered
    }
}

impl UartHal for EspUart {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, HalError> {
        Ok(self.buffered.read(buf).await)
    }

    async fn write(&mut self, buf: &[u8]) -> Result<usize, HalError> {
        self.buffered.write(buf).await;
        Ok(buf.len())
    }

    fn can_read(&self) -> bool {
        // Check if there's data in the inward pipe
        // This is a heuristic - actual implementation may need adjustment
        self.buffered.check_dropped_bytes() > 0 || self.configured
    }

    fn signal(&self) -> &Signal<CriticalSectionRawMutex, ()> {
        self.buffered.signal()
    }

    async fn reconfigure(&mut self, _config: UartConfig) -> Result<(), HalError> {
        // TODO: Implement runtime reconfiguration
        // Currently pins are configured at compile time
        self.configured = true;
        Ok(())
    }
}

/// Static storage for buffered UART
pub static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();

/// Signal for UART task synchronization  
pub static UART_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();

/// Initialize and get the buffered UART
pub async fn uart_buffer_wait_for_initialisation() -> &'static BufferedUart {
    UART_BUF.init_with(BufferedUart::new)
}

/// UART task for Embassy executor
#[embassy_executor::task]
pub async fn uart_task(
    uart_buf: &'static BufferedUart,
    uart1: UART1<'static>,
    pins: EspUartPins<'static>,
) {
    // Wait until SSH shell is ready
    UART_SIGNAL.wait().await;

    // Hardware UART setup
    let uart_config = Config::default().with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1),
    );

    let Ok(uart) = Uart::new(uart1, uart_config) else {
        return;
    };
    let uart = uart.with_rx(pins.rx).with_tx(pins.tx).into_async();

    // Run buffered TX/RX loop
    uart_buf.run(uart).await;
}
