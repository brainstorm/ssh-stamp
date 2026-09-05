// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! I2C master tunnelling over SSH.
//!
//! Provides the framing layer for the SSH `i2c` subsystem, mirroring the
//! CAN design in [`can`](crate::can): the protocol (parser + response
//! encoding) lives here, platform crates only execute the bus operations.
//! Unlike CAN there is no unsolicited traffic — the device is the bus
//! master, so everything is request/response.
//!
//! # Wire protocol (ASCII, one command per `\r`/`\n`-terminated line)
//!
//! All numbers are hexadecimal; addresses are 7-bit. At most
//! [`I2C_DATA_MAX`] data bytes per transfer.
//!
//! | Command      | Meaning                                     | Reply |
//! |--------------|---------------------------------------------|-------|
//! | `s`          | scan the bus (`0x08..=0x77`, 1-byte reads)  | found addresses, space-separated (empty line if none) |
//! | `wAADD...`   | write bytes `DD...` to address `AA`         | `OK` |
//! | `rAALL`      | read `LL` bytes from address `AA`           | the bytes, hex |
//! | `xAALLDD...` | write `DD...`, repeated-start read `LL`     | the bytes, hex |
//!
//! Failed transfers reply `NACK`, `TIMEOUT` or `ERR`; unparseable lines
//! reply `BADCMD`. Example: `w503A` writes `0x3A` to the device at `0x50`,
//! `x50021A` reads 2 bytes from register `0x1A` of the same device.

use core::fmt::Write as _;
use core::future::Future;

use embassy_futures::select::select;
use embedded_io_async::{Read, Write};
use log::debug;

/// Maximum data bytes in a single read or write transfer.
pub const I2C_DATA_MAX: usize = 32;

/// First and last 7-bit addresses probed by a bus scan (the reserved
/// address ranges are skipped, as `i2cdetect` does).
pub const SCAN_FIRST: u8 = 0x08;
pub const SCAN_LAST: u8 = 0x77;

/// Longest command line: `x` + 2 addr chars + 2 len chars + 64 data chars.
const LINE_SZ: usize = 5 + I2C_DATA_MAX * 2;

/// Upper bound for one encoded reply: a full scan finds
/// `SCAN_LAST - SCAN_FIRST + 1` = 112 addresses at 3 chars each, plus the
/// line terminator.
pub const RESPONSE_MAX: usize = 112 * 3 + 2;

/// Platform-agnostic buffered I2C master bridge.
///
/// Pumps protocol bytes between the SSH channel and the target I2C
/// peripheral. Every platform provides a concrete type implementing this
/// trait (ESP32: `ssh_stamp_esp32::BufferedI2c`).
pub trait BufferedI2c: Sync {
    fn read(&self, buf: &mut [u8]) -> impl Future<Output = usize>;
    fn write(&self, buf: &[u8]) -> impl Future<Output = ()>;

    /// Start-of-session hook: drop half-parsed command state left by a
    /// previous session and discard any stale buffered replies.
    fn reset_protocol(&self);
}

/// A decoded host command, ready for the platform to execute on the bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum I2cRequest {
    /// Probe `SCAN_FIRST..=SCAN_LAST` and report responding addresses.
    Scan,
    /// Write `data` to `addr`.
    Write {
        addr: u8,
        data: heapless::Vec<u8, I2C_DATA_MAX>,
    },
    /// Read `len` bytes from `addr`.
    Read { addr: u8, len: u8 },
    /// Write `data` to `addr`, then read `len` bytes with a repeated start.
    WriteRead {
        addr: u8,
        data: heapless::Vec<u8, I2C_DATA_MAX>,
        len: u8,
    },
    /// The line did not parse; reply [`I2cResponse::BadCommand`].
    Malformed,
}

/// Outcome of executing an [`I2cRequest`], encoded by [`encode_response`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum I2cResponse {
    /// Write completed.
    Ok,
    /// Bytes returned by a read or write-read.
    Data(heapless::Vec<u8, I2C_DATA_MAX>),
    /// Addresses that acknowledged during a scan.
    Scan(heapless::Vec<u8, 112>),
    /// The device did not acknowledge.
    Nack,
    /// The transfer timed out.
    Timeout,
    /// Any other bus error.
    Error,
    /// The command line did not parse.
    BadCommand,
}

/// Encode `resp` as one protocol line. Returns the number of bytes
/// written; `buf` must hold at least [`RESPONSE_MAX`] bytes.
pub fn encode_response(resp: &I2cResponse, buf: &mut [u8]) -> usize {
    let mut s = heapless::String::<RESPONSE_MAX>::new();
    match resp {
        I2cResponse::Ok => {
            let _ = s.push_str("OK");
        }
        I2cResponse::Data(data) => {
            for &b in data {
                let _ = write!(s, "{b:02X}");
            }
        }
        I2cResponse::Scan(addrs) => {
            for (i, &a) in addrs.iter().enumerate() {
                if i > 0 {
                    let _ = s.push(' ');
                }
                let _ = write!(s, "{a:02X}");
            }
        }
        I2cResponse::Nack => {
            let _ = s.push_str("NACK");
        }
        I2cResponse::Timeout => {
            let _ = s.push_str("TIMEOUT");
        }
        I2cResponse::Error => {
            let _ = s.push_str("ERR");
        }
        I2cResponse::BadCommand => {
            let _ = s.push_str("BADCMD");
        }
    }
    let _ = s.push_str("\r\n");
    let len = s.len().min(buf.len());
    buf[..len].copy_from_slice(&s.as_bytes()[..len]);
    len
}

/// Byte-stream front end for the SSH `i2c` subsystem: accumulates
/// `\r`/`\n`-terminated lines (SSH reads arrive fragmented) and decodes
/// them into [`I2cRequest`]s.
pub struct I2cParser {
    line: heapless::Vec<u8, LINE_SZ>,
    /// The current line overflowed [`LINE_SZ`]; report it as malformed at
    /// the next terminator instead of decoding a truncated command.
    overflow: bool,
}

impl I2cParser {
    #[must_use]
    pub fn new() -> Self {
        I2cParser {
            line: heapless::Vec::new(),
            overflow: false,
        }
    }

    /// Drop any half-accumulated command line.
    pub fn reset(&mut self) {
        self.line.clear();
        self.overflow = false;
    }

    /// Consume one host byte, returning a request once a line is complete.
    pub fn feed(&mut self, byte: u8) -> Option<I2cRequest> {
        if byte != b'\r' && byte != b'\n' {
            if self.line.push(byte).is_err() {
                self.overflow = true;
            }
            return None;
        }
        let request = if self.overflow {
            Some(I2cRequest::Malformed)
        } else if self.line.is_empty() {
            None
        } else {
            Some(decode_line(&self.line))
        };
        self.reset();
        request
    }
}

impl Default for I2cParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Decode one complete command line. `line` is never empty: [`I2cParser::feed`]
/// only calls this once a terminator arrives with bytes accumulated.
///
/// Every offset past the first byte goes through `get`, not indexing: a host
/// is free to send a truncated line (`w`, `x50`), and a panic here would take
/// the whole device down with it.
fn decode_line(line: &[u8]) -> I2cRequest {
    match line[0] {
        b's' if line.len() == 1 => I2cRequest::Scan,
        b'w' => match (hex_byte(line, 1), line.get(3..).and_then(hex_data)) {
            (Some(addr), Some(data)) if addr <= 0x7F && !data.is_empty() => {
                I2cRequest::Write { addr, data }
            }
            _ => I2cRequest::Malformed,
        },
        b'r' if line.len() == 5 => match (hex_byte(line, 1), hex_byte(line, 3)) {
            (Some(addr), Some(len))
                if addr <= 0x7F && len >= 1 && usize::from(len) <= I2C_DATA_MAX =>
            {
                I2cRequest::Read { addr, len }
            }
            _ => I2cRequest::Malformed,
        },
        b'x' => match (
            hex_byte(line, 1),
            hex_byte(line, 3),
            line.get(5..).and_then(hex_data),
        ) {
            (Some(addr), Some(len), Some(data))
                if addr <= 0x7F
                    && len >= 1
                    && usize::from(len) <= I2C_DATA_MAX
                    && !data.is_empty() =>
            {
                I2cRequest::WriteRead { addr, data, len }
            }
            _ => I2cRequest::Malformed,
        },
        _ => I2cRequest::Malformed,
    }
}

/// Parse the two hex chars at `line[at..at + 2]`.
fn hex_byte(line: &[u8], at: usize) -> Option<u8> {
    let s = line.get(at..at + 2)?;
    let s = core::str::from_utf8(s).ok()?;
    u8::from_str_radix(s, 16).ok()
}

/// Parse an even run of hex chars into bytes. A trailing half-byte leaves a
/// remainder, which is what rejects an odd-length run.
fn hex_data(chars: &[u8]) -> Option<heapless::Vec<u8, I2C_DATA_MAX>> {
    let (pairs, rest) = chars.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    let mut data = heapless::Vec::new();
    for pair in pairs {
        let s = core::str::from_utf8(pair).ok()?;
        data.push(u8::from_str_radix(s, 16).ok()?).ok()?;
    }
    Some(data)
}

/// Forwards an incoming SSH I2C channel to/from the local I2C bus, until
/// the connection drops.
///
/// # Errors
/// Returns an error if the SSH connection fails.
pub async fn i2c_bridge<I: BufferedI2c + ?Sized>(
    chan_read: impl Read<Error = sunset::Error>,
    chan_write: impl Write<Error = sunset::Error>,
    i2c: &I,
) -> Result<(), sunset::Error> {
    debug!("Starting I2C <--> SSH bridge");
    i2c.reset_protocol();
    select(i2c_to_ssh(i2c, chan_write), ssh_to_i2c(chan_read, i2c)).await;
    debug!("Stopping I2C <--> SSH bridge");
    Ok(())
}

async fn i2c_to_ssh<I: BufferedI2c + ?Sized>(
    i2c_buf: &I,
    mut chan_write: impl Write<Error = sunset::Error>,
) -> Result<(), sunset::Error> {
    let mut ssh_tx_buf = [0u8; 128];
    loop {
        let n = i2c_buf.read(&mut ssh_tx_buf).await;
        chan_write.write_all(&ssh_tx_buf[..n]).await?;
    }
}

async fn ssh_to_i2c<I: BufferedI2c + ?Sized>(
    mut chan_read: impl Read<Error = sunset::Error>,
    i2c_buf: &I,
) -> Result<(), sunset::Error> {
    let mut i2c_rx_buf = [0u8; 64];
    loop {
        let n = chan_read.read(&mut i2c_rx_buf).await?;
        if n == 0 {
            return Err(sunset::Error::ChannelEOF);
        }
        i2c_buf.write(&i2c_rx_buf[..n]).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a whole command line, terminator included, to a fresh parser.
    fn parse(line: &str) -> Option<I2cRequest> {
        let mut parser = I2cParser::new();
        let mut request = None;
        for byte in line.bytes().chain(core::iter::once(b'\n')) {
            request = parser.feed(byte).or(request);
        }
        request
    }

    fn data(bytes: &[u8]) -> heapless::Vec<u8, I2C_DATA_MAX> {
        heapless::Vec::from_slice(bytes).unwrap()
    }

    #[test]
    fn decodes_the_documented_commands() {
        assert_eq!(parse("s"), Some(I2cRequest::Scan));
        assert_eq!(
            parse("w503A"),
            Some(I2cRequest::Write {
                addr: 0x50,
                data: data(&[0x3A]),
            })
        );
        assert_eq!(
            parse("r5002"),
            Some(I2cRequest::Read { addr: 0x50, len: 2 })
        );
        assert_eq!(
            parse("x50021A"),
            Some(I2cRequest::WriteRead {
                addr: 0x50,
                data: data(&[0x1A]),
                len: 2,
            })
        );
    }

    /// A truncated line used to index past the end of the buffer, panicking
    /// the whole device on a one-character command.
    #[test]
    fn truncated_lines_are_malformed_not_a_panic() {
        for line in [
            "w", "w5", "w50", "x", "x5", "x50", "x500", "x5002", "r", "r50",
        ] {
            assert_eq!(
                parse(line),
                Some(I2cRequest::Malformed),
                "expected `{line}` to be rejected"
            );
        }
    }

    #[test]
    fn rejects_out_of_range_values() {
        // 8-bit address, odd hex run, and a read longer than the buffer.
        assert_eq!(parse("w8000"), Some(I2cRequest::Malformed));
        assert_eq!(parse("w50ABC"), Some(I2cRequest::Malformed));
        assert_eq!(parse("r5099"), Some(I2cRequest::Malformed));
        assert_eq!(parse("r5000"), Some(I2cRequest::Malformed));
        assert_eq!(parse("zzz"), Some(I2cRequest::Malformed));
        // `s` takes no arguments.
        assert_eq!(parse("s0"), Some(I2cRequest::Malformed));
    }

    #[test]
    fn empty_lines_yield_nothing() {
        assert_eq!(parse(""), None);
        let mut parser = I2cParser::new();
        assert_eq!(parser.feed(b'\r'), None);
        assert_eq!(parser.feed(b'\n'), None);
    }

    /// Overflowing the line buffer must report one malformed command at the
    /// next terminator, never a truncated (and silently different) transfer.
    #[test]
    fn overlong_lines_are_malformed() {
        let line = "w50".to_string() + &"AB".repeat(I2C_DATA_MAX + 8);
        assert_eq!(parse(&line), Some(I2cRequest::Malformed));
    }

    #[test]
    fn a_full_scan_reply_fits_the_response_buffer() {
        let addrs: heapless::Vec<u8, 112> = (SCAN_FIRST..=SCAN_LAST).collect();
        let mut buf = [0u8; RESPONSE_MAX];
        let len = encode_response(&I2cResponse::Scan(addrs), &mut buf);
        assert!(len < RESPONSE_MAX, "scan reply truncated at {len} bytes");
        assert!(buf[..len].ends_with(b"77\r\n"));
    }

    #[test]
    fn encodes_the_status_replies() {
        let mut buf = [0u8; RESPONSE_MAX];
        for (resp, expected) in [
            (I2cResponse::Ok, &b"OK\r\n"[..]),
            (I2cResponse::Nack, b"NACK\r\n"),
            (I2cResponse::Timeout, b"TIMEOUT\r\n"),
            (I2cResponse::Error, b"ERR\r\n"),
            (I2cResponse::BadCommand, b"BADCMD\r\n"),
            (I2cResponse::Data(data(&[0x0F, 0xA0])), b"0FA0\r\n"),
        ] {
            let len = encode_response(&resp, &mut buf);
            assert_eq!(&buf[..len], expected);
        }
    }
}
