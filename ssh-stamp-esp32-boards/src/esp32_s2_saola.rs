// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Espressif ESP32-S2-Saola-1 board support.
//!
//! Official board page:
//! <https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s2/esp32-s2-saola-1/index.html>

use crate::Board;

pub struct Esp32s2Saola;

impl Board for Esp32s2Saola {
    const NAME: &'static str = "esp32-s2-saola";
    const UART_RX: u8 = 10;
    const UART_TX: u8 = 11;
}
