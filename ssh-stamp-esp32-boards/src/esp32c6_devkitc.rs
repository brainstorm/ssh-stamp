// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Espressif ESP32-C6-DevKitC-1 board support.
//!
//! Official board page:
//! <https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/index.html>

use crate::Board;

pub struct Esp32c6Devkitc;

impl Board for Esp32c6Devkitc {
    const NAME: &'static str = "esp32c6-devkitc";
    const UART_RX: u8 = 10;
    const UART_TX: u8 = 11;
}
