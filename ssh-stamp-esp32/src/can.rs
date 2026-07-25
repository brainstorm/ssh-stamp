// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! CAN (TWAI) implementation for ESP32 family.
//!
//! Provides [`BufferedCan`] — a software-buffered, async CAN interface
//! satisfying [`ssh_stamp::can::BufferedCan`]. The bridge can pump the
//! same TWAI peripheral from two futures (TX and RX) concurrently because
//! both sides take `&self`.

use core::future::Future;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use esp_hal::gpio::AnyPin;
use esp_hal::peripherals::TWAI0;
use esp_hal::twai::{self, EspTwaiFrame, ExtendedId, StandardId, TwaiMode};
use log::warn;
use portable_atomic::{AtomicUsize, Ordering};
use ssh_stamp::can::{CanDecoder, CanEncoder, CanId, Slcan};
use static_cell::StaticCell;

const INWARD_BUF_SZ: usize = 256;
const OUTWARD_BUF_SZ: usize = 256;

/// Longest slcan line: `T` + 8 ID chars + 1 DLC char + 16 data chars.
const SLCAN_LINE_SZ: usize = 32;

/// Bidirectional pipe buffer between the TWAI peripheral and the SSH
/// `can` subsystem bridge. Frames travel slcan-encoded in both pipes.
pub struct BufferedCan {
    outward: Pipe<CriticalSectionRawMutex, OUTWARD_BUF_SZ>,
    inward: Pipe<CriticalSectionRawMutex, INWARD_BUF_SZ>,
    dropped_rx_frames: AtomicUsize,
    encoder: Slcan,
    decoder: Slcan,
}

impl BufferedCan {
    #[must_use]
    pub fn new() -> Self {
        BufferedCan {
            outward: Pipe::new(),
            inward: Pipe::new(),
            dropped_rx_frames: AtomicUsize::from(0),
            encoder: Slcan,
            decoder: Slcan,
        }
    }

    /// Transfer frames between the TWAI hardware and internal buffers.
    ///
    /// This should be awaited from an Embassy task run in an `InterruptExecutor`
    /// for lower latency.
    pub async fn run(&self, twai: twai::Twai<'static, esp_hal::Async>) {
        let (mut twai_rx, mut twai_tx) = twai.split();

        loop {
            use embassy_futures::select::select;

            let rd_from = async {
                let mut frame_buf = [0u8; SLCAN_LINE_SZ];
                loop {
                    let frame = match twai_rx.receive_async().await {
                        Ok(frame) => frame,
                        Err(e) => {
                            warn!("TWAI RX error: {e:?}");
                            continue;
                        }
                    };
                    let n = self.encoder.encode(&frame, &mut frame_buf);
                    // Drop whole frames when the SSH side isn't keeping up:
                    // a partial slcan line would corrupt the stream.
                    if self.inward.free_capacity() < n {
                        let _ = self.dropped_rx_frames.fetch_update(
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                            |d| Some(d.saturating_add(1)),
                        );
                    } else {
                        self.inward.write_all(&frame_buf[..n]).await;
                    }
                }
            };

            let rd_to = async {
                // SSH reads arrive fragmented, so reassemble slcan lines
                // before decoding. Oversized lines are discarded until the
                // next terminator resyncs the stream.
                let mut line = heapless::Vec::<u8, SLCAN_LINE_SZ>::new();
                let mut chunk = [0u8; 64];
                loop {
                    let n = self.outward.read(&mut chunk).await;
                    for &byte in &chunk[..n] {
                        if byte != b'\r' && byte != b'\n' {
                            if line.push(byte).is_err() {
                                line.clear();
                            }
                            continue;
                        }
                        if let Some(frame) = self.decoder.decode(&line) {
                            let id: Option<twai::Id> = match frame.id {
                                CanId::Standard(id) => StandardId::new(id).map(twai::Id::from),
                                CanId::Extended(id) => ExtendedId::new(id).map(twai::Id::from),
                            };
                            if let Some(esp_frame) =
                                id.and_then(|id| EspTwaiFrame::new(id, &frame.data))
                                && let Err(e) = twai_tx.transmit_async(&esp_frame).await
                            {
                                warn!("TWAI TX error: {e:?}");
                            }
                        }
                        line.clear();
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

    /// Number of frames the RX side dropped since the last call. Resets the counter.
    pub fn check_dropped_frames(&self) -> usize {
        self.dropped_rx_frames.swap(0, Ordering::Relaxed)
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

/// Route GPIO19/20 to the onboard TJA1051 CAN transceiver.
///
/// On the Waveshare ESP32-S3-Touch-LCD-4.3 those pins are shared with USB
/// through an FSUSB42UMX analog switch, steered by the `USB_SEL` line
/// (EXIO5) of a CH422G I2C IO expander. Driving `USB_SEL` high selects CAN.
///
/// # Panics
///
/// Panics if the I2C peripheral cannot be initialised.
#[cfg(feature = "board-esp32-s3-touch-lcd-43")]
pub fn route_can_transceiver(
    i2c0: esp_hal::peripherals::I2C0<'static>,
    sda: AnyPin<'static>,
    scl: AnyPin<'static>,
) {
    use esp_hal::i2c::master::I2c;
    use log::debug;

    // CH422G: direction register at address 0x24, output register at 0x38.
    // 0x20 raises only USB_SEL (bit 5); the other EXIO lines (TP_RST,
    // LCD_BL, LCD_RST, SD_CS) stay low — ssh-stamp does not use the LCD.
    let mut i2c = I2c::new(i2c0, esp_hal::i2c::master::Config::default())
        .expect("I2C init error")
        .with_sda(sda)
        .with_scl(scl);
    if let Err(e) = i2c.write(0x24, &[0x01]) {
        warn!("CH422G direction write failed: {e:?}");
    }
    if let Err(e) = i2c.write(0x38, &[0x20]) {
        warn!("CH422G output write failed: {e:?}");
    }
    debug!("CH422G USB_SEL set: GPIO19/20 routed to CAN transceiver");
}

/// Embassy task that owns the hardware TWAI peripheral and pumps it
/// through [`BufferedCan::run`]. Spawn from a higher-priority
/// `InterruptExecutor` for lower latency.
#[embassy_executor::task]
pub async fn can_task(
    can_buf: &'static BufferedCan,
    twai0: TWAI0<'static>,
    pins: EspCanPins<'static>,
) {
    let twai_config = twai::TwaiConfiguration::new(
        twai0,
        pins.rx,
        pins.tx,
        twai::BaudRate::B500K,
        TwaiMode::Normal,
    );

    let twai = twai_config.into_async().start();
    can_buf.run(twai).await;
}
