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
use esp_println::dbg;
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
                    let n = uart_rx.read_async(&mut uart_rx_buf).await.unwrap();

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
    software_reset();
}
// use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

pub async fn uart_disable() -> () {
    // disable uart
    software_reset();
}

#[derive(Default)]
pub struct GPIOS<'a> {
    pub gpio9: Option<AnyPin<'a>>,
    pub gpio10: Option<AnyPin<'a>>,
    pub gpio11: Option<AnyPin<'a>>,
    pub gpio12: Option<AnyPin<'a>>,
    pub gpio13: Option<AnyPin<'a>>,
    pub gpio14: Option<AnyPin<'a>>,
    pub gpio20: Option<AnyPin<'a>>,
    pub gpio21: Option<AnyPin<'a>>,
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
    gpios: GPIOS<'static>,
) -> () {
    dbg!("UART task started");

    // Wait until ssh shell complete
    UART_SIGNAL.wait().await;
    dbg!("UART signal recieved");

    // Temporarily hardcoded pin numbers. Restore once ServEvent::SessionEnv properly updates config
    // let config_lock = config.lock().await;
    // let rx: u8 = config_lock.uart_pins.rx;
    // let tx: u8 = config_lock.uart_pins.tx;
    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32")] {
            let rx: u8 = 13;
            let tx: u8 = 14;
        } else if #[cfg(feature = "esp32c2")] {
            let rx: u8 = 9;
            let tx: u8 = 10;
        } else if #[cfg(feature = "esp32c3")] {
            let rx: u8 = 20;
            let tx: u8 = 21;
        } else {
            let rx: u8 = 10;
            let tx: u8 = 11;
        }
    );

    println!("Config Read - RX Pin: {}  TX Pin: {}", rx, tx);
    if rx != tx {
        let mut _holder9 = Some(gpios.gpio9);
        let mut _holder10 = Some(gpios.gpio10);
        let mut _holder11 = Some(gpios.gpio11);
        let mut _holder13 = Some(gpios.gpio13);
        let mut _holder14 = Some(gpios.gpio14);
        let mut _holder20 = Some(gpios.gpio20);
        let mut _holder21 = Some(gpios.gpio21);
        // Not every GPIO is available for every target.
        // TODO: Merge all targets to only match on available GPIO
        cfg_if::cfg_if!(
            if #[cfg(feature = "esp32")] {
                let rx_pin = match rx {
                    13 => _holder13.take().unwrap().unwrap(),
                    14 => _holder14.take().unwrap().unwrap(),
                    _ => _holder13.take().unwrap().unwrap(),
                };
                let tx_pin = match tx {
                    13 => _holder13.take().unwrap().unwrap(),
                    14 => _holder14.take().unwrap().unwrap(),
                    _ => _holder13.take().unwrap().unwrap(),
                };
            } else if #[cfg(feature = "esp32c2")] {
                let rx_pin = match rx {
                    9 => _holder9.take().unwrap().unwrap(),
                    10 => _holder10.take().unwrap().unwrap(),
                    _ => _holder9.take().unwrap().unwrap(),
                };
                let tx_pin = match tx {
                    9 => _holder9.take().unwrap().unwrap(),
                    10 => _holder10.take().unwrap().unwrap(),
                    _ => _holder10.take().unwrap().unwrap(),
                };
            } else if #[cfg(feature = "esp32c3")] {
                let rx_pin = match rx {
                    20 => _holder20.take().unwrap().unwrap(),
                    21 => _holder21.take().unwrap().unwrap(),
                    _ => _holder20.take().unwrap().unwrap(),
                };
                let tx_pin = match tx {
                    20 => _holder20.take().unwrap().unwrap(),
                    21 => _holder21.take().unwrap().unwrap(),
                    _ => _holder21.take().unwrap().unwrap(),
                };
            } else {
                let rx_pin = match rx {
                    10 => _holder10.take().unwrap().unwrap(),
                    11 => _holder11.take().unwrap().unwrap(),
                    _ => _holder10.take().unwrap().unwrap(),
                };
                let tx_pin = match tx {
                    10 => _holder10.take().unwrap().unwrap(),
                    11 => _holder11.take().unwrap().unwrap(),
                    _ => _holder11.take().unwrap().unwrap(),
                };
            }
        );

        // Hardware UART setup
        dbg!("UART config");
        let uart_config = Config::default().with_rx(
            RxConfig::default()
                .with_fifo_full_threshold(16)
                .with_timeout(1),
        );

        dbg!("UART setup pins");
        let uart = Uart::new(uart1, uart_config)
            .unwrap()
            .with_rx(rx_pin)
            .with_tx(tx_pin)
            .into_async();

        // Run the main buffered TX/RX loop
        dbg!("uart_task running UART");
        uart_buf.run(uart).await;
    }
    // TODO: Pin config error
    dbg!("uart_task Pin config error! Using the same pin number for RX and TX!");
    ()
}
