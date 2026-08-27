// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! UART implementation for ESP32 family.
//!
//! Provides [`BufferedUart`] — a software-buffered, async, full-duplex UART
//! satisfying [`ssh_stamp::serial::BufferedSerial`]. The bridge can poll the
//! same UART from two futures (TX and RX) concurrently because both sides
//! take `&self`.

use core::future::Future;

use embassy_executor::SendSpawner;
use embassy_sync::pipe::TryWriteError;
use embassy_sync::signal::Signal;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use esp_hal::Async;
use esp_hal::gpio::AnyPin;
#[cfg(feature = "bench-loopback")]
use esp_hal::gpio::Flex;
use esp_hal::peripherals::UART1;
use esp_hal::uart::{Config, DataBits, Parity, RxConfig, StopBits, Uart};
use portable_atomic::{AtomicUsize, Ordering};
use ssh_stamp::serial::BufferedSerial;
use ssh_stamp_hal::{Parity as LineParity, UartParams};
use static_cell::StaticCell;

const INWARD_BUF_SZ: usize = 512;
const OUTWARD_BUF_SZ: usize = 256;
const UART_BUF_SZ: usize = 64;

/// The ESP32 UART peripherals reject anything above 5 Mbaud.
const MAX_BAUD: u32 = 5_000_000;

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

                    // This must take into consideration the length returned by `write_async`,
                    // as it may be less than the full buffer. Follow-up loop iterations
                    // then write any remainder.
                    let mut tx_slice = &tx_buf[..n];
                    while !tx_slice.is_empty() {
                        let Ok(written) = uart_tx.write_async(tx_slice).await else {
                            break;
                        };

                        tx_slice = &tx_slice[written..];
                    }
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
///
/// The pin numbers inside come from the selected board's TOML in the
/// `ssh-stamp-esp32-boards` crate; its front page carries the generated pin
/// catalog for every board of this platform.
pub struct EspUartPins<'a> {
    pub rx: AnyPin<'a>,
    pub tx: AnyPin<'a>,
}

/// Static storage for the buffered UART singleton.
pub static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();

/// Signal raised by [`ssh_stamp::platform::PlatformServices::activate_uart`]
/// to release [`uart_task`] from its initial wait.
pub static UART_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();

/// Translates the persisted, target-agnostic [`UartParams`] into an esp-hal
/// [`Config`].
///
/// Values the peripheral cannot honour fall back to the 8N1 default instead of
/// refusing to bring the bridge up, so a stale or corrupt stored config still
/// leaves a usable serial console.
fn esp_uart_config(params: UartParams) -> Config {
    let data_bits = match params.data_bits {
        5 => DataBits::_5,
        6 => DataBits::_6,
        7 => DataBits::_7,
        _ => DataBits::_8,
    };
    let parity = match params.parity {
        LineParity::Even => Parity::Even,
        LineParity::Odd => Parity::Odd,
        LineParity::None => Parity::None,
    };
    let stop_bits = if params.stop_bits == 2 {
        StopBits::_2
    } else {
        StopBits::_1
    };

    Config::default()
        .with_baudrate(params.baud.clamp(1, MAX_BAUD))
        .with_data_bits(data_bits)
        .with_parity(parity)
        .with_stop_bits(stop_bits)
}

/// Embassy task that owns the hardware UART and pumps it through
/// [`BufferedUart::run`]. Spawn from a higher-priority `InterruptExecutor`
/// for lower latency.
///
/// `params` are the line settings from the device config, applied here since
/// the UART is configured once for the lifetime of the boot.
#[embassy_executor::task]
pub async fn uart_task(
    uart_buf: &'static BufferedUart,
    uart1: UART1<'static>,
    pins: EspUartPins<'static>,
    params: UartParams,
) {
    UART_SIGNAL.wait().await;

    let uart_config = esp_uart_config(params).with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1),
    );

    let uart = Uart::new(uart1, uart_config).expect("UART config error");

    // Route the TX back into the RX input to measure round trip.
    #[cfg(feature = "bench-loopback")]
    let uart = {
        log::warn!("bench-loopback active, the TX is looped back to RX.");
        let (rx_sig, tx_sig) = Flex::new(pins.tx).split();
        uart.with_rx(rx_sig).with_tx(tx_sig).into_async()
    };
    #[cfg(not(feature = "bench-loopback"))]
    let uart = uart.with_rx(pins.rx).with_tx(pins.tx).into_async();

    uart_buf.run(uart).await;
}

/// Creates the [`BufferedUart`] singleton and spawns [`uart_task`] on the
/// given spawner, returning the buffer the rest of the system talks to. The
/// firmware feeds it the spawner from
/// [`start_interrupt_executor`](crate::start_interrupt_executor), so the
/// task runs at interrupt priority. The task waits on [`UART_SIGNAL`] before
/// touching the hardware.
///
/// # Panics
///
/// Panics if called more than once per boot: the [`BufferedUart`] singleton
/// and the task can each only be created once.
pub fn spawn_uart(
    spawner: SendSpawner,
    uart1: UART1<'static>,
    pins: EspUartPins<'static>,
    params: UartParams,
) -> &'static BufferedUart {
    let uart_buf = UART_BUF.init_with(BufferedUart::new);
    spawner.spawn(uart_task(uart_buf, uart1, pins, params).expect("uart_task spawn failed"));
    uart_buf
}
