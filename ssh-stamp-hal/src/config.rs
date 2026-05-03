// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Hardware configuration types.

/// UART peripheral configuration.
#[derive(Clone, Debug)]
pub struct UartConfig {
    pub tx_pin: u8,
    pub rx_pin: u8,
    pub cts_pin: Option<u8>,
    pub rts_pin: Option<u8>,
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

/// `WiFi` access point configuration.
///
/// Contains settings for running the device as a `WiFi` access point.
#[derive(Clone, Debug)]
pub struct WifiApConfigStatic {
    /// Network name (SSID), max 32 characters.
    pub ssid: heapless::String<32>,
    /// Optional WPA2 password, max 63 characters.
    pub password: Option<heapless::String<63>>,
    /// `WiFi` channel (1-14 for 2.4GHz).
    pub channel: u8,
    /// MAC address for the access point interface.
    pub mac: [u8; 6],
}

impl Default for WifiApConfigStatic {
    fn default() -> Self {
        Self {
            ssid: heapless::String::new(),
            password: None,
            channel: 1,
            mac: [0; 6],
        }
    }
}
