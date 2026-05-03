// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Hardware configuration types.
//!
//! This module provides configuration structures for initializing HAL peripherals.

/// Top-level hardware configuration.
///
/// Contains all peripheral-specific configurations needed to initialize
/// a complete hardware platform.
#[derive(Clone, Debug)]
pub struct HardwareConfig {
    /// UART configuration.
    pub uart: UartConfig,
    /// `WiFi` access point configuration.
    pub wifi: WifiApConfigStatic,
}

/// UART peripheral configuration.
///
/// Defines pin assignments and baud rate for a UART interface.
#[derive(Clone, Debug)]
pub struct UartConfig {
    /// TX pin number.
    pub tx_pin: u8,
    /// RX pin number.
    pub rx_pin: u8,
    /// CTS (Clear To Send) pin for hardware flow control.
    pub cts_pin: Option<u8>,
    /// RTS (Ready To Send) pin for hardware flow control.
    pub rts_pin: Option<u8>,
    /// Baud rate in bits per second.
    pub baud_rate: u32,
}

impl Default for UartConfig {
    fn default() -> Self {
        Self {
            tx_pin: 0,
            rx_pin: 0,
            cts_pin: None,
            rts_pin: None,
            baud_rate: 115_200,
        }
    }
}

/// `WiFi` access point configuration (static).
///
/// Contains settings for running the device as a `WiFi` access point.
/// Uses `heapless::String` for `no_std` compatibility.
///
/// SSID and password should always be randomly generated — never use
/// predictable or empty defaults.
#[derive(Clone, Debug)]
pub struct WifiApConfigStatic {
    /// Network name (SSID), max 32 characters. Must be randomly generated.
    pub ssid: heapless::String<32>,
    /// WPA2 password, max 63 characters. Must be randomly generated.
    pub password: heapless::String<63>,
    /// MAC address for the access point interface.
    pub mac: [u8; 6],
}
