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
//! # Polled, not interrupt-driven
//!
//! `bl616-wifi`'s UART has no interrupt path, so the bridge polls. The FIFO
//! is 32 bytes, which at 115200 baud is about 2.7 ms of data, so a 1 ms idle
//! poll has margin; the loop only sleeps when a read came back empty, so a
//! busy line is drained as fast as the executor will run it.
//!
//! # Overflow drops the oldest, and says so
//!
//! When nothing is attached to the SSH end, the inward pipe fills. Dropping
//! the *oldest* bytes keeps the most recent output, which is what someone
//! attaching to a console wants to see, and `check_dropped_bytes` reports the
//! loss so the session can say so rather than silently showing a gap.

use core::sync::atomic::{AtomicUsize, Ordering};

use bl616_wifi::uart::{Config, Uart};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::{Pipe, TryWriteError};
use embassy_sync::signal::Signal;
use ssh_stamp::serial::BufferedSerial;
use static_cell::StaticCell;

/// Buffered in each direction.
const INWARD_BUF_SZ: usize = 2048;
const OUTWARD_BUF_SZ: usize = 512;
/// Bytes moved per poll.
const CHUNK: usize = 64;

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
    pub async fn run(&self, mut uart: Uart) -> ! {
        let mut rx = [0u8; CHUNK];
        let mut tx = [0u8; CHUNK];

        loop {
            let mut idle = true;

            let n = uart.read(&mut rx);
            if n > 0 {
                idle = false;
                self.push_inward(&rx[..n]);
            }

            // Anything the SSH side has queued goes out. try_read so a silent
            // client never blocks the receive direction.
            if let Ok(n) = self.outward.try_read(&mut tx)
                && n > 0
            {
                idle = false;
                uart.write(&tx[..n]);
            }

            if idle {
                embassy_time::Timer::after_millis(1).await;
            } else {
                // Let other tasks run between chunks; a saturated line must
                // not starve the network stack.
                embassy_futures::yield_now().await;
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
