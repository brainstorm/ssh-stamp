// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

/// Wrapper around bidirectional embassy-sync Pipes, in order to handle UART
/// RX/RX happening in an InterruptExecutor at higher priority.
///
/// Doesn't implement the InterruptExecutor, in the task in the app should await
/// the 'run' async function.
///
use crate::config::SSHStampConfig;
use embassy_futures::select::select;
use embassy_sync::pipe::TryWriteError;
use embassy_sync::signal::Signal;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use esp_hal::Async;
use esp_hal::gpio::AnyPin;
use esp_hal::peripherals::UART1;
use esp_hal::system::software_reset;
use esp_hal::uart::{Config, RxConfig, Uart};
use esp_println::println;
use portable_atomic::{AtomicUsize, Ordering};
use static_cell::StaticCell;
use sunset_async::SunsetMutex;

// Sizes of the software buffers. Inward is more
// important as an overrun here drops bytes. A full outward
// buffer will only block the executor.
const INWARD_BUF_SZ: usize = 512;
const OUTWARD_BUF_SZ: usize = 256;

// Size of the buffer for hardware read/write ops.
const UART_BUF_SZ: usize = 64;

/// Bidirectional pipe buffer for UART communications
pub struct BufferedUart {
    outward: Pipe<CriticalSectionRawMutex, OUTWARD_BUF_SZ>,
    inward: Pipe<CriticalSectionRawMutex, INWARD_BUF_SZ>,
    dropped_rx_bytes: AtomicUsize,
}

pub struct UartConfig {}

impl BufferedUart {
    pub fn new() -> Self {
        BufferedUart {
            outward: Pipe::new(),
            inward: Pipe::new(),
            dropped_rx_bytes: AtomicUsize::from(0),
        }
    }

    /// Transfer data between the UART and the buffer struct.
    ///
    /// This should be awaited from an Embassy task that's run
    /// in an InterruptExecutor for lower latency.
    pub async fn run(&self, uart: Uart<'_, Async>) {
        let (mut uart_rx, mut uart_tx) = uart.split();
        let mut uart_rx_buf = [0u8; UART_BUF_SZ];
        let mut uart_tx_buf = [0u8; UART_BUF_SZ];

        loop {
            let rd_from = async {
                loop {
                    // Note: println! is intentionally avoided here as this runs in an
                    // InterruptExecutor at high priority. Blocking I/O would cause scheduler panics.
                    let Ok(n) = uart_rx.read_async(&mut uart_rx_buf).await else {
                        continue;
                    };

                    let mut rx_slice = &uart_rx_buf[..n];

                    // Write rx_slice to 'inward' pipe, dropping bytes rather than blocking if
                    // the pipe is full
                    while !rx_slice.is_empty() {
                        rx_slice = match self.inward.try_write(rx_slice) {
                            Ok(w) => &rx_slice[w..],
                            Err(TryWriteError::Full) => {
                                // If the receive buffer is full (no SSH client, or network congestion) then
                                // drop the oldest bytes from the pipe so we can still write the newest ones.
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
                    // TODO: handle write errors
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

    /// Return the number of dropped bytes (if any) since the last check,
    /// and reset the internal count to 0.
    pub fn check_dropped_bytes(&self) -> usize {
        self.dropped_rx_bytes.swap(0, Ordering::Relaxed)
    }

    pub fn reconfigure(&self, _config: &'static SunsetMutex<SSHStampConfig>) {
        todo!();
    }
}

impl Default for BufferedUart {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn uart_buffer_disable() -> () {
    // disable uart buffer
    println!("UART buffer disabled");
    // TODO: Correctly disable/restart UART buffer and/or send messsage to user over SSH
    software_reset();
}
// use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

pub async fn uart_disable() -> () {
    // disable uart
    println!("UART disabled");
    // TODO: Correctly disable/restart UART and/or send messsage to user over SSH
    software_reset();
}

/// UART pins for the buffered UART task.
///
/// Pins are selected at compile time based on the target chip.
/// Each target only populates the pins it actually uses.
pub struct UartPins<'a> {
    pub rx: AnyPin<'a>,
    pub tx: AnyPin<'a>,
}

pub static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();
pub static UART_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();

pub async fn uart_buffer_wait_for_initialisation() -> &'static BufferedUart {
    UART_BUF.init_with(BufferedUart::new)
}

#[embassy_executor::task]
pub async fn uart_task(
    uart_buf: &'static BufferedUart,
    uart1: UART1<'static>,
    _config: &'static SunsetMutex<SSHStampConfig>,
    pins: UartPins<'static>,
) {
    // Note: dbg!/println! avoided throughout as this task runs in an InterruptExecutor
    // at high priority where blocking I/O can cause scheduler panics.

    // Wait until ssh shell complete
    UART_SIGNAL.wait().await;

    // Hardware UART setup - pins are already selected at compile time
    let uart_config = Config::default().with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1),
    );

    // Silent failure to avoid println! in interrupt context
    let Ok(uart) = Uart::new(uart1, uart_config) else {
        return;
    };
    let uart = uart.with_rx(pins.rx).with_tx(pins.tx).into_async();

    // Run the main buffered TX/RX loop
    uart_buf.run(uart).await;
}
