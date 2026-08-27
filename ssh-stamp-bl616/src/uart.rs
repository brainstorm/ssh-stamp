// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The serial port ssh-stamp bridges to an SSH channel.
//!
//! # Status
//!
//! Not wired to hardware yet. The BL616's UART0 is currently the vendor
//! console — `bl616-wifi`'s `usb-console` feature moves the *console* to USB
//! but does not hand the UART over — so bridging it needs the pins claimed
//! away from the SDK first. Until that happens this reads nothing and drops
//! what it is given, which makes an SSH session connect and stay silent
//! rather than fail in a way that looks like a network fault.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use ssh_stamp::serial::BufferedSerial;

/// Raised when an SSH session attaches, so a port can start the UART lazily.
pub static UART_SIGNAL: Signal<CriticalSectionRawMutex, u32> = Signal::new();

/// A serial bridge that is not attached to anything yet.
pub struct Bl616Serial;

impl BufferedSerial for Bl616Serial {
    async fn read(&self, _buf: &mut [u8]) -> usize {
        // Park rather than spin: returning 0 in a loop would busy-wait the
        // executor for a device that is never going to speak.
        core::future::pending::<()>().await;
        0
    }

    async fn write(&self, _buf: &[u8]) {}

    fn check_dropped_bytes(&self) -> usize {
        0
    }
}
