// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! CAN (TWAI) implementation for ESP32 family.
//!
//! Provides [`BufferedCan`] — a software-buffered, async CAN interface
//! satisfying [`ssh_stamp::can::BufferedCan`]. The bridge can pump the
//! same TWAI peripheral from two futures (TX and RX) concurrently because
//! both sides take `&self`. All framing (slcan / GVRET auto-detection)
//! lives in the platform-agnostic [`ssh_stamp::can`] layer; this file only
//! moves bytes and frames.

use core::future::Future;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use embassy_time::{Duration, with_timeout};
use esp_hal::gpio::AnyPin;
use esp_hal::peripherals::TWAI0;
use esp_hal::twai::{self, EspTwaiFrame, ExtendedId, StandardId, TwaiMode};
use log::warn;
use portable_atomic::{AtomicBool, AtomicUsize, Ordering};
use ssh_stamp::can::{CanAction, CanId, CanParser, ENCODED_FRAME_MAX, encode_frame};
use static_cell::StaticCell;

const INWARD_BUF_SZ: usize = 512;
const OUTWARD_BUF_SZ: usize = 256;

/// Bus bitrate in bit/s. Keep in sync with the `BaudRate` passed to the
/// TWAI driver in [`can_task`]; also reported to GVRET clients.
const CAN_BITRATE: u32 = 500_000;

/// TWAI operating mode.
///
/// `Normal` is what a real bus needs: the controller takes part in
/// acknowledgement and retransmits a frame until some node acknowledges it.
/// On a bench with no acknowledging peer (only a scope/analyzer attached)
/// that same rule makes a single write repeat on the wire, so the
/// `can-no-ack` feature switches to `SelfTest` mode, which drops the
/// acknowledgement requirement: one slcan line, exactly one frame.
#[cfg(feature = "can-no-ack")]
const TWAI_MODE: TwaiMode = TwaiMode::SelfTest;
#[cfg(not(feature = "can-no-ack"))]
const TWAI_MODE: TwaiMode = TwaiMode::Normal;

/// Safety net for transmissions stuck retrying (shorted/unwired bus, or a
/// frame nothing acknowledges in `Normal` mode). Dropping the transmit
/// future on timeout issues a TWAI TX-abort, cancelling the pending
/// retransmissions instead of retrying forever. In `Normal` mode the
/// timeout is generous so arbitration on a busy but healthy bus never
/// drops frames; with `can-no-ack` any wait at all means a bus fault.
#[cfg(feature = "can-no-ack")]
const TX_TIMEOUT: Duration = Duration::from_millis(1);
#[cfg(not(feature = "can-no-ack"))]
const TX_TIMEOUT: Duration = Duration::from_millis(10);

/// Bidirectional pipe buffer between the TWAI peripheral and the SSH
/// `can` subsystem bridge. Traffic in both pipes is framed by the codec
/// layer (slcan lines or GVRET binary messages).
pub struct BufferedCan {
    outward: Pipe<CriticalSectionRawMutex, OUTWARD_BUF_SZ>,
    inward: Pipe<CriticalSectionRawMutex, INWARD_BUF_SZ>,
    dropped_rx_frames: AtomicUsize,
    /// Bus→host framing: GVRET binary after the host sent `0xE7`,
    /// slcan ASCII otherwise. Reset at the start of every session.
    binary_mode: AtomicBool,
    /// Set by [`BufferedCan::reset_protocol`]; makes the pump task drop
    /// parser state left over from a previous session.
    proto_reset: AtomicBool,
}

impl BufferedCan {
    #[must_use]
    pub fn new() -> Self {
        BufferedCan {
            outward: Pipe::new(),
            inward: Pipe::new(),
            dropped_rx_frames: AtomicUsize::from(0),
            binary_mode: AtomicBool::new(false),
            proto_reset: AtomicBool::new(false),
        }
    }

    /// Transfer frames between the TWAI hardware and internal buffers.
    ///
    /// This should be awaited from an Embassy task run in an `InterruptExecutor`
    /// for lower latency.
    ///
    /// Both directions write into `inward` (encoded bus frames and GVRET
    /// replies). That is safe from interleaving because they run in this
    /// single task and only issue a `write_all` after checking the whole
    /// message fits, so the write never yields midway.
    pub async fn run(&self, twai: twai::Twai<'static, esp_hal::Async>) {
        let (mut twai_rx, mut twai_tx) = twai.split();

        loop {
            use embassy_futures::select::select;

            let rd_from = async {
                let mut frame_buf = [0u8; ENCODED_FRAME_MAX];
                loop {
                    let frame = match twai_rx.receive_async().await {
                        Ok(frame) => frame,
                        Err(e) => {
                            warn!("TWAI RX error: {e:?}");
                            continue;
                        }
                    };
                    let binary = self.binary_mode.load(Ordering::Relaxed);
                    let n = encode_frame(&frame, binary, &mut frame_buf);
                    self.send_to_ssh(&frame_buf[..n]).await;
                }
            };

            let rd_to = async {
                let mut parser = CanParser::new(CAN_BITRATE);
                let mut chunk = [0u8; 64];
                loop {
                    let n = self.outward.read(&mut chunk).await;
                    if self.proto_reset.swap(false, Ordering::Relaxed) {
                        parser.reset();
                    }
                    for &byte in &chunk[..n] {
                        match parser.feed(byte) {
                            None => {}
                            Some(CanAction::EnableBinary) => {
                                self.binary_mode.store(true, Ordering::Relaxed);
                            }
                            Some(CanAction::Reply(bytes)) => {
                                self.send_to_ssh(&bytes).await;
                            }
                            Some(CanAction::Transmit(frame)) => {
                                let id: Option<twai::Id> = match frame.id {
                                    CanId::Standard(id) => StandardId::new(id).map(twai::Id::from),
                                    CanId::Extended(id) => ExtendedId::new(id).map(twai::Id::from),
                                };
                                let Some(esp_frame) =
                                    id.and_then(|id| EspTwaiFrame::new(id, &frame.data))
                                else {
                                    continue;
                                };
                                match with_timeout(TX_TIMEOUT, twai_tx.transmit_async(&esp_frame))
                                    .await
                                {
                                    Ok(Ok(())) => {}
                                    Ok(Err(e)) => warn!("TWAI TX error: {e:?}"),
                                    Err(_) => warn!(
                                        "TWAI TX stuck (bus fault or missing ACK), aborting retransmission"
                                    ),
                                }
                            }
                        }
                    }
                }
            };

            select(rd_from, rd_to).await;
        }
    }

    /// Queue one whole encoded message for the SSH side, or drop it (and
    /// count the drop) when the session isn't keeping up: a partial slcan
    /// line or GVRET message would corrupt the stream.
    async fn send_to_ssh(&self, msg: &[u8]) {
        if self.inward.free_capacity() < msg.len() {
            let _ =
                self.dropped_rx_frames
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |d| {
                        Some(d.saturating_add(1))
                    });
        } else {
            self.inward.write_all(msg).await;
        }
    }

    pub async fn read(&self, buf: &mut [u8]) -> usize {
        self.inward.read(buf).await
    }

    pub async fn write(&self, buf: &[u8]) {
        self.outward.write_all(buf).await;
    }

    /// Number of frames the RX side dropped since the last call. Resets the counter.
    pub fn check_dropped_frames(&self) -> usize {
        self.dropped_rx_frames.swap(0, Ordering::Relaxed)
    }

    /// Start-of-session reset: back to slcan framing, drop half-parsed
    /// protocol state and discard bus traffic buffered while no session
    /// was attached.
    pub fn reset_protocol(&self) {
        self.binary_mode.store(false, Ordering::Relaxed);
        self.proto_reset.store(true, Ordering::Relaxed);
        let mut sink = [0u8; 32];
        while self.inward.try_read(&mut sink).is_ok() {}
    }
}

impl Default for BufferedCan {
    fn default() -> Self {
        Self::new()
    }
}

impl ssh_stamp::can::BufferedCan for BufferedCan {
    fn read(&self, buf: &mut [u8]) -> impl Future<Output = usize> {
        BufferedCan::read(self, buf)
    }

    fn write(&self, buf: &[u8]) -> impl Future<Output = ()> {
        BufferedCan::write(self, buf)
    }

    fn check_dropped_frames(&self) -> usize {
        BufferedCan::check_dropped_frames(self)
    }

    fn reset_protocol(&self) {
        BufferedCan::reset_protocol(self);
    }
}

/// CAN pins configuration.
///
/// The pin numbers inside are target-specific and come from the board's
/// TOML in the `ssh-stamp-esp32-boards` crate.
pub struct EspCanPins<'a> {
    pub tx: AnyPin<'a>,
    pub rx: AnyPin<'a>,
}

/// Static storage for the buffered CAN singleton.
pub static CAN_BUF: StaticCell<BufferedCan> = StaticCell::new();

/// Embassy task that owns the hardware TWAI peripheral and pumps it
/// through [`BufferedCan::run`]. Spawn from a higher-priority
/// `InterruptExecutor` for lower latency.
#[embassy_executor::task]
pub async fn can_task(
    can_buf: &'static BufferedCan,
    twai0: TWAI0<'static>,
    pins: EspCanPins<'static>,
) {
    let twai_config =
        twai::TwaiConfiguration::new(twai0, pins.rx, pins.tx, twai::BaudRate::B500K, TWAI_MODE);

    let twai = twai_config.into_async().start();
    can_buf.run(twai).await;
}
