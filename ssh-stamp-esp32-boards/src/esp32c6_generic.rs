// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Generic ESP32-C6 board support.
//!
//! Intended for custom ESP32-C6 boards that follow the standard C6 pinout
//! but are not the Espressif DevKitC-1. Uses GPIO 10/11 for UART, matching
//! the safe default on the ESP32-C6 family.
//!
//! See the ESP32-C6 datasheet for pin multiplexing constraints:
//! <https://www.espressif.com/en/support/documents/technical-documents>

use crate::Board;

pub struct Esp32c6Generic;

impl Board for Esp32c6Generic {
    const NAME: &'static str = "esp32c6-generic";
    const UART_RX: u8 = 10;
    const UART_TX: u8 = 11;
}
