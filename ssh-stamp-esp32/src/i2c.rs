// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! I2C master implementation for ESP32 family.
//!
//! Provides [`BufferedI2c`] — a software-buffered, async I2C master
//! satisfying [`ssh_stamp::i2c::BufferedI2c`]. All protocol details
//! (command parsing, response encoding) live in the platform-agnostic
//! [`ssh_stamp::i2c`] layer; this file only executes bus operations.
//! The device is the bus master, so unlike CAN there is no unsolicited
//! traffic and no `select` in the pump: one request, one reply.

use core::future::Future;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use esp_hal::gpio::AnyPin;
use esp_hal::i2c::master::{Config, Error, I2c};
use esp_hal::peripherals::I2C0;
use portable_atomic::{AtomicBool, Ordering};
use ssh_stamp::i2c::{
    I2C_DATA_MAX, I2cParser, I2cRequest, I2cResponse, RESPONSE_MAX, SCAN_FIRST, SCAN_LAST,
    encode_response,
};
use static_cell::StaticCell;

const INWARD_BUF_SZ: usize = 256;
const OUTWARD_BUF_SZ: usize = 256;

/// Bidirectional pipe buffer between the I2C peripheral and the SSH
/// `i2c` subsystem bridge. `outward` carries command lines from the host,
/// `inward` carries the replies.
pub struct BufferedI2c {
    outward: Pipe<CriticalSectionRawMutex, OUTWARD_BUF_SZ>,
    inward: Pipe<CriticalSectionRawMutex, INWARD_BUF_SZ>,
    /// Set by [`BufferedI2c::reset_protocol`]; makes the pump task drop
    /// parser state left over from a previous session.
    proto_reset: AtomicBool,
}

impl BufferedI2c {
    #[must_use]
    pub fn new() -> Self {
        BufferedI2c {
            outward: Pipe::new(),
            inward: Pipe::new(),
            proto_reset: AtomicBool::new(false),
        }
    }

    /// Parse host commands, execute them on the bus and queue the replies.
    ///
    /// This should be awaited from an Embassy task run in an
    /// `InterruptExecutor` for lower latency.
    pub async fn run(&self, mut i2c: I2c<'static, esp_hal::Async>) {
        let mut parser = I2cParser::new();
        let mut chunk = [0u8; 64];
        let mut reply_buf = [0u8; RESPONSE_MAX];
        loop {
            let n = self.outward.read(&mut chunk).await;
            if self.proto_reset.swap(false, Ordering::Relaxed) {
                parser.reset();
            }
            for &byte in &chunk[..n] {
                let Some(request) = parser.feed(byte) else {
                    continue;
                };
                let response = execute(&mut i2c, &request).await;
                let len = encode_response(&response, &mut reply_buf);
                self.inward.write_all(&reply_buf[..len]).await;
            }
        }
    }

    pub async fn read(&self, buf: &mut [u8]) -> usize {
        self.inward.read(buf).await
    }

    pub async fn write(&self, buf: &[u8]) {
        self.outward.write_all(buf).await;
    }

    /// Start-of-session reset: drop half-parsed command state and discard
    /// replies buffered for a previous session.
    pub fn reset_protocol(&self) {
        self.proto_reset.store(true, Ordering::Relaxed);
        let mut sink = [0u8; 32];
        while self.inward.try_read(&mut sink).is_ok() {}
    }
}

/// Execute one decoded request on the bus and map the outcome.
async fn execute(i2c: &mut I2c<'static, esp_hal::Async>, request: &I2cRequest) -> I2cResponse {
    match request {
        I2cRequest::Scan => {
            // Probe with 1-byte reads: esp-hal rejects zero-length
            // transfers (`Error::ZeroLengthInvalid`), and a read is the
            // least intrusive probe for most devices.
            let mut found = heapless::Vec::new();
            let mut scratch = [0u8; 1];
            for addr in SCAN_FIRST..=SCAN_LAST {
                if i2c.read_async(addr, &mut scratch).await.is_ok() {
                    let _ = found.push(addr);
                }
            }
            I2cResponse::Scan(found)
        }
        I2cRequest::Write { addr, data } => match i2c.write_async(*addr, data).await {
            Ok(()) => I2cResponse::Ok,
            Err(e) => error_response(e),
        },
        I2cRequest::Read { addr, len } => {
            let mut data = heapless::Vec::<u8, I2C_DATA_MAX>::new();
            data.resize_default(usize::from(*len)).unwrap_or_default();
            match i2c.read_async(*addr, &mut data).await {
                Ok(()) => I2cResponse::Data(data),
                Err(e) => error_response(e),
            }
        }
        I2cRequest::WriteRead { addr, data, len } => {
            let mut read = heapless::Vec::<u8, I2C_DATA_MAX>::new();
            read.resize_default(usize::from(*len)).unwrap_or_default();
            match i2c.write_read_async(*addr, data, &mut read).await {
                Ok(()) => I2cResponse::Data(read),
                Err(e) => error_response(e),
            }
        }
        I2cRequest::Malformed => I2cResponse::BadCommand,
    }
}

fn error_response(e: Error) -> I2cResponse {
    match e {
        Error::AcknowledgeCheckFailed(_) => I2cResponse::Nack,
        Error::Timeout => I2cResponse::Timeout,
        _ => I2cResponse::Error,
    }
}

impl Default for BufferedI2c {
    fn default() -> Self {
        Self::new()
    }
}

impl ssh_stamp::i2c::BufferedI2c for BufferedI2c {
    fn read(&self, buf: &mut [u8]) -> impl Future<Output = usize> {
        BufferedI2c::read(self, buf)
    }

    fn write(&self, buf: &[u8]) -> impl Future<Output = ()> {
        BufferedI2c::write(self, buf)
    }

    fn reset_protocol(&self) {
        BufferedI2c::reset_protocol(self);
    }
}

/// I2C pins configuration.
///
/// The pin numbers inside are target-specific and come from the board's
/// TOML in the `ssh-stamp-esp32-boards` crate.
pub struct EspI2cPins<'a> {
    pub sda: AnyPin<'a>,
    pub scl: AnyPin<'a>,
}

/// Static storage for the buffered I2C singleton.
pub static I2C_BUF: StaticCell<BufferedI2c> = StaticCell::new();

/// Embassy task that owns the hardware I2C master and pumps it through
/// [`BufferedI2c::run`]. The bus runs at the esp-hal default speed
/// (100 kHz standard mode).
#[embassy_executor::task]
pub async fn i2c_task(
    i2c_buf: &'static BufferedI2c,
    i2c0: I2C0<'static>,
    pins: EspI2cPins<'static>,
) {
    let i2c = I2c::new(i2c0, Config::default())
        .expect("I2C config error")
        .with_sda(pins.sda)
        .with_scl(pins.scl)
        .into_async();

    i2c_buf.run(i2c).await;
}
