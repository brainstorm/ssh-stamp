// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! CAN frame encoding/decoding for SSH tunnelling.
//!
//! Provides a trait-based framing layer so the encapsulation format
//! (slcan, GVRET, etc.) can be swapped without touching the SSH bridge.
//! Uses `embedded-can` traits as the frame abstraction.

use core::fmt::Write as _;
use core::future::Future;

use embassy_futures::select::select;
use embedded_can::{Frame, Id};
use embedded_io_async::{Read, Write};
use log::{debug, warn};

/// Encodes a CAN frame into a byte buffer for transmission over SSH.
pub trait CanEncoder {
    /// Encode `frame` into `buf`. Returns the number of bytes written.
    fn encode(&self, frame: &impl Frame, buf: &mut [u8]) -> usize;
}

/// Decodes a byte buffer into a CAN frame.
pub trait CanDecoder {
    /// Try to decode a CAN frame from `buf`. Returns `Some(frame)` on success,
    /// `None` if the buffer does not contain a complete frame.
    fn decode(&self, buf: &[u8]) -> Option<CanFrame>;
}

/// Platform-agnostic buffered CAN bridge.
///
/// The CAN bridge pumps slcan-encoded frames between the SSH channel and
/// the target CAN peripheral. Every platform provides a concrete type
/// implementing this trait (ESP32: `ssh_stamp_esp32::BufferedCan`).
pub trait BufferedCan: Sync {
    fn read(&self, buf: &mut [u8]) -> impl Future<Output = usize>;
    fn write(&self, buf: &[u8]) -> impl Future<Output = ()>;
    fn check_dropped_frames(&self) -> usize;
}

/// A simple owned CAN frame for use in the platform-agnostic layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanFrame {
    pub id: CanId,
    pub data: heapless::Vec<u8, 8>,
}

/// CAN identifier, mirroring `embedded_can::Id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanId {
    Standard(u16),
    Extended(u32),
}

impl From<Id> for CanId {
    fn from(id: Id) -> Self {
        match id {
            Id::Standard(s) => CanId::Standard(s.as_raw()),
            Id::Extended(e) => CanId::Extended(e.as_raw()),
        }
    }
}

/// slcan (ASCII) encoder/decoder.
///
/// Encodes frames as `tIIILDD...\r` (standard) or `TIIIIIIIIDD...\r` (extended),
/// where I = hex ID, L = data length, D = hex data bytes.
pub struct Slcan;

impl CanEncoder for Slcan {
    fn encode(&self, frame: &impl Frame, buf: &mut [u8]) -> usize {
        let mut s = heapless::String::<64>::new();
        match frame.id() {
            Id::Standard(id) => {
                let _ = write!(s, "t{:03X}{:1X}", id.as_raw(), frame.dlc());
            }
            Id::Extended(id) => {
                let _ = write!(s, "T{:08X}{:1X}", id.as_raw(), frame.dlc());
            }
        }
        let data = frame.data();
        for &b in data {
            let _ = write!(s, "{b:02X}");
        }
        let _ = s.push('\r');
        let len = s.len().min(buf.len());
        buf[..len].copy_from_slice(s.as_bytes());
        len
    }
}

impl CanDecoder for Slcan {
    fn decode(&self, buf: &[u8]) -> Option<CanFrame> {
        let s = core::str::from_utf8(buf).ok()?;
        // Byte-index slicing below is only safe on ASCII input.
        if !s.is_ascii() {
            return None;
        }
        let s = s.trim_end_matches('\r');
        let bytes = s.as_bytes();
        match bytes.first()? {
            b't' => {
                if s.len() < 5 {
                    return None;
                }
                let id = u16::from_str_radix(&s[1..4], 16).ok()?;
                let dlc = (bytes[4] as char).to_digit(16)? as usize;
                if dlc > 8 {
                    return None;
                }
                let mut data = heapless::Vec::new();
                let hex_data = &s[5..];
                if hex_data.len() < dlc * 2 {
                    return None;
                }
                for i in 0..dlc {
                    let b = u8::from_str_radix(&hex_data[i * 2..i * 2 + 2], 16).ok()?;
                    let _ = data.push(b);
                }
                Some(CanFrame {
                    id: CanId::Standard(id),
                    data,
                })
            }
            b'T' => {
                if s.len() < 10 {
                    return None;
                }
                let id = u32::from_str_radix(&s[1..9], 16).ok()?;
                let dlc = (bytes[9] as char).to_digit(16)? as usize;
                if dlc > 8 {
                    return None;
                }
                let mut data = heapless::Vec::new();
                let hex_data = &s[10..];
                if hex_data.len() < dlc * 2 {
                    return None;
                }
                for i in 0..dlc {
                    let b = u8::from_str_radix(&hex_data[i * 2..i * 2 + 2], 16).ok()?;
                    let _ = data.push(b);
                }
                Some(CanFrame {
                    id: CanId::Extended(id),
                    data,
                })
            }
            _ => None,
        }
    }
}

/// Forwards an incoming SSH CAN channel to/from the local CAN bus, until
/// the connection drops.
///
/// # Errors
/// Returns an error if the SSH connection fails.
pub async fn can_bridge<C: BufferedCan + ?Sized>(
    chan_read: impl Read<Error = sunset::Error>,
    chan_write: impl Write<Error = sunset::Error>,
    can: &C,
) -> Result<(), sunset::Error> {
    debug!("Starting CAN <--> SSH bridge");
    select(can_to_ssh(can, chan_write), ssh_to_can(chan_read, can)).await;
    debug!("Stopping CAN <--> SSH bridge");
    Ok(())
}

async fn can_to_ssh<C: BufferedCan + ?Sized>(
    can_buf: &C,
    mut chan_write: impl Write<Error = sunset::Error>,
) -> Result<(), sunset::Error> {
    let mut ssh_tx_buf = [0u8; 128];
    loop {
        let dropped = can_buf.check_dropped_frames();
        if dropped > 0 {
            warn!("CAN RX dropped {dropped} frames");
        }
        let n = can_buf.read(&mut ssh_tx_buf).await;
        chan_write.write_all(&ssh_tx_buf[..n]).await?;
    }
}

async fn ssh_to_can<C: BufferedCan + ?Sized>(
    mut chan_read: impl Read<Error = sunset::Error>,
    can_buf: &C,
) -> Result<(), sunset::Error> {
    let mut can_tx_buf = [0u8; 64];
    loop {
        let n = chan_read.read(&mut can_tx_buf).await?;
        if n == 0 {
            return Err(sunset::Error::ChannelEOF);
        }
        can_buf.write(&can_tx_buf[..n]).await;
    }
}
