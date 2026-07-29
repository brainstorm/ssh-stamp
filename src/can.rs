// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! CAN frame encoding/decoding for SSH tunnelling.
//!
//! Provides a trait-based framing layer so the encapsulation format can be
//! swapped without touching the SSH bridge. Two encapsulations are
//! implemented, multiplexed over the same SSH `can` subsystem byte stream
//! by [`CanParser`] — the same auto-detection real GVRET hardware performs
//! on its serial port:
//!
//! * **slcan** (ASCII, the default): one `t`/`T` frame per `\r`-terminated
//!   line. For interactive `ssh -s ... can` use and slcan-speaking tools.
//! * **GVRET** (binary): the `SavvyCAN` / ESP32RET wire protocol. `0xF1`
//!   starts a command (send frame, time sync, device info, keepalive, ...)
//!   and is answered with binary replies; `0xE7` (sent twice by `SavvyCAN` on
//!   connect) switches bus→host frame output to GVRET framing.
//!
//! `embedded-can` traits are used as the frame abstraction.

use core::fmt::Write as _;
use core::future::Future;

use embassy_futures::select::select;
use embassy_time::Instant;
use embedded_can::{ExtendedId, Frame, Id, StandardId};
use embedded_io_async::{Read, Write};
use log::{debug, warn};

/// Longest slcan line: `T` + 8 ID chars + 1 DLC char + 16 data chars.
const SLCAN_LINE_SZ: usize = 32;

/// Upper bound for one encoded frame or protocol reply in either
/// encapsulation (slcan frame: 27 bytes, GVRET frame: 20 bytes, largest
/// GVRET reply: 17 bytes).
pub const ENCODED_FRAME_MAX: usize = 32;

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
/// The CAN bridge pumps encoded frames between the SSH channel and
/// the target CAN peripheral. Every platform provides a concrete type
/// implementing this trait (ESP32: `ssh_stamp_esp32::BufferedCan`).
pub trait BufferedCan: Sync {
    fn read(&self, buf: &mut [u8]) -> impl Future<Output = usize>;
    fn write(&self, buf: &[u8]) -> impl Future<Output = ()>;
    fn check_dropped_frames(&self) -> usize;

    /// Start-of-session hook: revert bus→host framing to slcan (ASCII),
    /// reset any half-parsed protocol state left by a previous session and
    /// discard bus traffic buffered while no session was attached.
    fn reset_protocol(&self);
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

impl From<CanId> for Id {
    fn from(id: CanId) -> Self {
        match id {
            CanId::Standard(v) => Id::Standard(StandardId::new(v).unwrap_or(StandardId::ZERO)),
            CanId::Extended(v) => Id::Extended(ExtendedId::new(v).unwrap_or(ExtendedId::ZERO)),
        }
    }
}

impl Frame for CanFrame {
    fn new(id: impl Into<Id>, data: &[u8]) -> Option<Self> {
        Some(CanFrame {
            id: id.into().into(),
            data: heapless::Vec::from_slice(data).ok()?,
        })
    }

    // Remote (RTR) frames are not tunnelled.
    fn new_remote(_id: impl Into<Id>, _dlc: usize) -> Option<Self> {
        None
    }

    fn is_extended(&self) -> bool {
        matches!(self.id, CanId::Extended(_))
    }

    fn is_remote_frame(&self) -> bool {
        false
    }

    fn id(&self) -> Id {
        self.id.into()
    }

    fn dlc(&self) -> usize {
        self.data.len()
    }

    fn data(&self) -> &[u8] {
        &self.data
    }
}

/// slcan (ASCII) encoder/decoder.
///
/// Encodes frames as `tIIILDD...\r` (standard) or `TIIIIIIIILDD...\r`
/// (extended), where I = hex ID, L = data length, D = hex data bytes.
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

/// GVRET (binary) encoder for bus→host frames.
///
/// Message layout, as produced by ESP32RET and parsed by `SavvyCAN`:
/// `F1 00 <timestamp us, u32 LE> <id, u32 LE, bit31 = extended>`
/// `<dlc | bus << 4> <data...> 00`.
pub struct Gvret;

impl CanEncoder for Gvret {
    fn encode(&self, frame: &impl Frame, buf: &mut [u8]) -> usize {
        let data = frame.data();
        let n = 12 + data.len();
        let Ok(dlc) = u8::try_from(data.len()) else {
            return 0;
        };
        if dlc > 8 || buf.len() < n {
            return 0;
        }
        let raw_id = match frame.id() {
            Id::Standard(id) => u32::from(id.as_raw()),
            Id::Extended(id) => id.as_raw() | 0x8000_0000,
        };
        buf[0] = 0xF1;
        buf[1] = gvret::BUILD_CAN_FRAME;
        buf[2..6].copy_from_slice(&timestamp_us());
        buf[6..10].copy_from_slice(&raw_id.to_le_bytes());
        buf[10] = dlc; // bus 0 in the high nibble
        buf[11..11 + data.len()].copy_from_slice(data);
        buf[11 + data.len()] = 0; // checksum placeholder, ignored by SavvyCAN
        n
    }
}

/// Microsecond timestamp in GVRET's format: a `u32` that wraps by design
/// (every ~71 minutes), little endian.
fn timestamp_us() -> [u8; 4] {
    #[allow(clippy::cast_possible_truncation)]
    let t = Instant::now().as_micros() as u32;
    t.to_le_bytes()
}

/// Encode a bus→host frame in whichever framing the session negotiated:
/// GVRET binary after the host sent `0xE7`, slcan ASCII otherwise.
pub fn encode_frame(frame: &impl Frame, binary: bool, buf: &mut [u8]) -> usize {
    if binary {
        Gvret.encode(frame, buf)
    } else {
        Slcan.encode(frame, buf)
    }
}

/// GVRET command bytes (the byte following an `0xF1` marker).
mod gvret {
    pub const BUILD_CAN_FRAME: u8 = 0x00;
    pub const TIME_SYNC: u8 = 0x01;
    pub const GET_DIG_INPUTS: u8 = 0x02;
    pub const GET_ANALOG_INPUTS: u8 = 0x03;
    pub const SET_DIG_OUT: u8 = 0x04;
    pub const SETUP_CANBUS: u8 = 0x05;
    pub const GET_CANBUS_PARAMS: u8 = 0x06;
    pub const GET_DEVICE_INFO: u8 = 0x07;
    pub const SET_SINGLEWIRE_MODE: u8 = 0x08;
    pub const KEEPALIVE: u8 = 0x09;
    pub const SET_SYSTYPE: u8 = 0x0A;
    pub const ECHO_CAN_FRAME: u8 = 0x0B;
    pub const GET_NUM_BUSES: u8 = 0x0C;
    pub const GET_EXT_BUSES: u8 = 0x0D;
    pub const SET_EXT_BUSES: u8 = 0x0E;
}

/// What the platform pump should do with the bytes just parsed.
pub enum CanAction {
    /// A complete host→bus frame was decoded; transmit it.
    Transmit(CanFrame),
    /// A GVRET command produced a reply; send it back to the host.
    Reply(heapless::Vec<u8, ENCODED_FRAME_MAX>),
    /// The host sent `0xE7`: switch bus→host framing to GVRET binary.
    EnableBinary,
}

/// In-progress GVRET command, after the `0xF1` marker.
#[derive(Clone, Copy)]
enum GvretState {
    /// Waiting for the command byte.
    Command,
    /// Collecting a host→bus frame: id(4) bus(1) dlc(1) data(dlc) chk(1).
    /// `echo` replies the frame to the host instead of transmitting it.
    Frame {
        echo: bool,
        buf: [u8; 15],
        got: usize,
    },
    /// Swallow N payload bytes of a command we accept but ignore.
    Consume(usize),
}

/// Byte-stream front end for the SSH `can` subsystem.
///
/// Feeds one byte at a time; slcan lines are accumulated and decoded here
/// (SSH reads arrive fragmented), GVRET commands are parsed by a state
/// machine mirroring the ESP32RET firmware. `0xF1`/`0xE7` outside a GVRET
/// command abort any partial slcan line — they cannot occur in valid slcan.
pub struct CanParser {
    /// Bus bitrate reported to GVRET clients (bit/s).
    bitrate: u32,
    line: heapless::Vec<u8, SLCAN_LINE_SZ>,
    gvret: Option<GvretState>,
}

impl CanParser {
    #[must_use]
    pub fn new(bitrate: u32) -> Self {
        CanParser {
            bitrate,
            line: heapless::Vec::new(),
            gvret: None,
        }
    }

    /// Drop any half-parsed slcan line or GVRET command.
    pub fn reset(&mut self) {
        self.line.clear();
        self.gvret = None;
    }

    /// Consume one host byte, returning an action once one is complete.
    pub fn feed(&mut self, byte: u8) -> Option<CanAction> {
        if let Some(state) = self.gvret.take() {
            return self.feed_gvret(state, byte);
        }
        match byte {
            0xF1 => {
                self.line.clear();
                self.gvret = Some(GvretState::Command);
                None
            }
            0xE7 => {
                self.line.clear();
                Some(CanAction::EnableBinary)
            }
            b'\r' | b'\n' => {
                let frame = Slcan.decode(&self.line);
                self.line.clear();
                frame.map(CanAction::Transmit)
            }
            b => {
                // Oversized/garbage lines are discarded until the next
                // terminator resyncs the stream.
                if self.line.push(b).is_err() {
                    self.line.clear();
                }
                None
            }
        }
    }

    fn feed_gvret(&mut self, state: GvretState, byte: u8) -> Option<CanAction> {
        match state {
            GvretState::Command => self.gvret_command(byte),
            GvretState::Consume(n) => {
                if n > 1 {
                    self.gvret = Some(GvretState::Consume(n - 1));
                }
                None
            }
            GvretState::Frame { echo, mut buf, got } => {
                buf[got] = byte;
                let got = got + 1;
                if got >= 6 {
                    let dlc = usize::from(buf[5] & 0x0F).min(8);
                    // Header (6) + data + one trailing checksum byte
                    // (transmitted but never verified, as in ESP32RET).
                    if got == 6 + dlc + 1 {
                        return Self::finish_frame(echo, &buf, dlc);
                    }
                }
                self.gvret = Some(GvretState::Frame { echo, buf, got });
                None
            }
        }
    }

    fn gvret_command(&mut self, cmd: u8) -> Option<CanAction> {
        match cmd {
            gvret::BUILD_CAN_FRAME | gvret::ECHO_CAN_FRAME => {
                self.gvret = Some(GvretState::Frame {
                    echo: cmd == gvret::ECHO_CAN_FRAME,
                    buf: [0u8; 15],
                    got: 0,
                });
                None
            }
            gvret::TIME_SYNC => {
                let mut b = [0u8; 6];
                b[0] = 0xF1;
                b[1] = gvret::TIME_SYNC;
                b[2..6].copy_from_slice(&timestamp_us());
                reply(&b)
            }
            // No digital/analog IO is exposed; report zeroed inputs.
            gvret::GET_DIG_INPUTS => reply(&[0xF1, gvret::GET_DIG_INPUTS, 0, 0]),
            gvret::GET_ANALOG_INPUTS => {
                reply(&[0xF1, gvret::GET_ANALOG_INPUTS, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            }
            gvret::GET_CANBUS_PARAMS => {
                // Bus 0 enabled at the fixed hardware bitrate, bus 1 absent.
                let mut b = [0u8; 12];
                b[0] = 0xF1;
                b[1] = gvret::GET_CANBUS_PARAMS;
                b[2] = 1;
                b[3..7].copy_from_slice(&self.bitrate.to_le_bytes());
                b[8..12].copy_from_slice(&self.bitrate.to_le_bytes());
                reply(&b)
            }
            gvret::GET_DEVICE_INFO => {
                // Build number 618 (0x026A) — SavvyCAN only displays it.
                reply(&[0xF1, gvret::GET_DEVICE_INFO, 0x6A, 0x02, 0, 0, 0, 0])
            }
            gvret::KEEPALIVE => reply(&[0xF1, gvret::KEEPALIVE, 0xDE, 0xAD]),
            gvret::GET_NUM_BUSES => reply(&[0xF1, gvret::GET_NUM_BUSES, 1]),
            gvret::GET_EXT_BUSES => {
                // No SWCAN/LIN buses: 15 zeroed payload bytes.
                let mut b = [0u8; 17];
                b[0] = 0xF1;
                b[1] = gvret::GET_EXT_BUSES;
                reply(&b)
            }
            // Accepted but ignored: the bus runs at a fixed bitrate and has
            // no digital outputs / single-wire / system-type settings.
            gvret::SET_DIG_OUT | gvret::SET_SINGLEWIRE_MODE | gvret::SET_SYSTYPE => {
                self.gvret = Some(GvretState::Consume(1));
                None
            }
            gvret::SETUP_CANBUS => {
                self.gvret = Some(GvretState::Consume(8));
                None
            }
            gvret::SET_EXT_BUSES => {
                self.gvret = Some(GvretState::Consume(10));
                None
            }
            _ => None,
        }
    }

    fn finish_frame(echo: bool, buf: &[u8; 15], dlc: usize) -> Option<CanAction> {
        let raw_id = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let id = if raw_id & 0x8000_0000 != 0 {
            CanId::Extended(raw_id & 0x1FFF_FFFF)
        } else {
            CanId::Standard((raw_id & 0x7FF) as u16)
        };
        let frame = CanFrame {
            id,
            data: heapless::Vec::from_slice(&buf[6..6 + dlc]).ok()?,
        };
        if echo {
            let mut b = [0u8; ENCODED_FRAME_MAX];
            let n = Gvret.encode(&frame, &mut b);
            reply(&b[..n])
        } else {
            Some(CanAction::Transmit(frame))
        }
    }
}

fn reply(bytes: &[u8]) -> Option<CanAction> {
    heapless::Vec::from_slice(bytes).ok().map(CanAction::Reply)
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
    can.reset_protocol();
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
