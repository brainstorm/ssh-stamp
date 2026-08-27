// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The serial port ssh-stamp bridges to an SSH channel.
//!
//! Two pipes and a task that moves bytes between them and UART0: `inward`
//! carries what the device said, `outward` what the SSH client typed. Same
//! shape as the ESP port, because the interesting behaviour is in the
//! overflow policy rather than the plumbing.
//!
//! # Nothing here polls
//!
//! `bl616-wifi`'s UART is interrupt-driven, so both directions of the bridge
//! wait on a waker: the loop selects between "the handler received
//! something" and "the SSH side queued something", and runs at neither the
//! line rate nor a tick.
//!
//! # Which pins, and which line settings
//!
//! The pins come from the board's TOML in `ssh-stamp-bl616-boards` and the
//! line settings from the stored config, both by way of [`uart_config`]. The
//! port is configured once at boot, so a change to either takes effect on the
//! next one. They are routed to UART0 when the port is opened, which
//! is the only thing that routes them at all in this build: the vendor SDK
//! muxes UART0 only when it puts its own console there, and here the console
//! is on USB.
//!
//! # Overflow drops the oldest, and says so
//!
//! When nothing is attached to the SSH end, the inward pipe fills. Dropping
//! the *oldest* bytes keeps the most recent output, which is what someone
//! attaching to a console wants to see, and `check_dropped_bytes` reports the
//! loss so the session can say so rather than silently showing a gap.

use core::sync::atomic::{AtomicUsize, Ordering};

use bl616_wifi::uart::{Config, DataBits, Parity, StopBits, Uart};
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::{Pipe, TryWriteError};
use embassy_sync::signal::Signal;
use ssh_stamp::serial::BufferedSerial;
use ssh_stamp_hal::{Parity as LineParity, UartParams};
use static_cell::StaticCell;

/// Buffered in each direction.
const INWARD_BUF_SZ: usize = 2048;
const OUTWARD_BUF_SZ: usize = 512;
/// Bytes moved at a time in either direction.
const CHUNK: usize = 64;

/// The baud rates the peripheral can express, as the vendor divides its
/// 40 MHz clock: below the floor the divisor no longer fits the register, and
/// the ceiling is the fastest rate the vendor's own examples run.
const MIN_BAUD: u32 = 1_200;
const MAX_BAUD: u32 = 2_000_000;

/// Raised when an SSH session attaches, so the port is not opened until
/// something is listening.
pub static UART_SIGNAL: Signal<CriticalSectionRawMutex, u8> = Signal::new();

/// The bridge, shared between the task and the SSH session.
pub static UART_BUF: StaticCell<Bl616Serial> = StaticCell::new();

/// A serial port with a buffer on each side.
pub struct Bl616Serial {
    /// Device to SSH.
    inward: Pipe<CriticalSectionRawMutex, INWARD_BUF_SZ>,
    /// SSH to device.
    outward: Pipe<CriticalSectionRawMutex, OUTWARD_BUF_SZ>,
    dropped_rx_bytes: AtomicUsize,
}

impl Default for Bl616Serial {
    fn default() -> Self {
        Self::new()
    }
}

impl Bl616Serial {
    #[must_use]
    pub const fn new() -> Self {
        Bl616Serial {
            inward: Pipe::new(),
            outward: Pipe::new(),
            dropped_rx_bytes: AtomicUsize::new(0),
        }
    }

    /// Move bytes between the pipes and the hardware. Never returns.
    ///
    /// Both arms are cancel-safe — neither consumes anything until it is
    /// ready to hand it over — so losing the race costs nothing.
    pub async fn run(&self, mut uart: Uart) -> ! {
        let mut rx = [0u8; CHUNK];
        let mut tx = [0u8; CHUNK];

        loop {
            match select(uart.read(&mut rx), self.outward.read(&mut tx)).await {
                Either::First(n) => {
                    // Bytes the driver's own ring could not hold are lost
                    // before this sees them, and are counted in the same
                    // place so the session reports every gap it has.
                    self.dropped_rx_bytes
                        .fetch_add(uart.overruns(), Ordering::Relaxed);
                    self.push_inward(&rx[..n]);
                }
                Either::Second(n) => uart.write(&tx[..n]).await,
            }
        }
    }

    /// Append to the inward pipe, discarding the oldest bytes when full.
    fn push_inward(&self, mut data: &[u8]) {
        while !data.is_empty() {
            match self.inward.try_write(data) {
                Ok(written) => data = &data[written..],
                Err(TryWriteError::Full) => {
                    // Make room by dropping the oldest, and count it.
                    let mut waste = [0u8; CHUNK];
                    let want = data.len().min(waste.len());
                    let dropped = self.inward.try_read(&mut waste[..want]).unwrap_or(0);
                    self.dropped_rx_bytes.fetch_add(dropped, Ordering::Relaxed);
                    if dropped == 0 {
                        // Nothing could be freed; drop the rest rather than
                        // spin forever.
                        self.dropped_rx_bytes
                            .fetch_add(data.len(), Ordering::Relaxed);
                        return;
                    }
                }
            }
        }
    }
}

impl BufferedSerial for Bl616Serial {
    async fn read(&self, buf: &mut [u8]) -> usize {
        self.inward.read(buf).await
    }

    async fn write(&self, buf: &[u8]) {
        self.outward.write_all(buf).await;
    }

    fn check_dropped_bytes(&self) -> usize {
        self.dropped_rx_bytes.swap(0, Ordering::Relaxed)
    }
}

/// How to open UART0 on this board.
///
/// The pins come from the board's TOML in `ssh-stamp-bl616-boards`; the line
/// settings are the persisted, target-agnostic [`UartParams`].
///
/// Values the peripheral cannot honour are clamped or fall back to the 8N1
/// default rather than refusing to bring the bridge up, so a stale or corrupt
/// stored config still leaves a usable serial console.
#[must_use]
pub fn uart_config(params: UartParams) -> Config {
    Config {
        baudrate: params.baud.clamp(MIN_BAUD, MAX_BAUD),
        data_bits: match params.data_bits {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            _ => DataBits::Eight,
        },
        parity: match params.parity {
            LineParity::Even => Parity::Even,
            LineParity::Odd => Parity::Odd,
            LineParity::None => Parity::None,
        },
        stop_bits: if params.stop_bits == 2 {
            StopBits::Two
        } else {
            StopBits::One
        },
        rx_pin: Some(ssh_stamp_bl616_boards::UART_RX),
        tx_pin: Some(ssh_stamp_bl616_boards::UART_TX),
    }
}

/// Open UART0 once a session attaches, then bridge it forever.
///
/// Waiting for the signal matters: UART0 is shared with the vendor console
/// when the firmware is not built with `usb-console`, and claiming it at boot
/// would swallow the startup log of a build that is still being brought up.
#[embassy_executor::task]
pub async fn uart_task(serial: &'static Bl616Serial, config: Config) {
    UART_SIGNAL.wait().await;

    match Uart::open(&config) {
        Ok(uart) => serial.run(uart).await,
        Err(e) => {
            bl616_wifi::println!("[ssh-stamp] uart0 unavailable: {e}");
        }
    }
}
