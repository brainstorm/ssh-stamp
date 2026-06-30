// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Hardware configuration types.

use heapless::String;

/// UART peripheral configuration.
///
/// Pin numbers (`tx_pin`, `rx_pin`) are target-specific and must be set by
/// the port binary before use. There are no cross-platform default values;
/// each port crate defines pin assignments in its `src/bin/` entry point.
/// See the `ssh-stamp-esp32` binary's module documentation for ESP32 defaults.
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

/// `WiFi` band mode for the access point.
///
/// Selects whether the AP operates on 2.4GHz, 5GHz, or both.
/// Only the ESP32-C5 supports 5GHz; other chips ignore the setting
/// and always operate on 2.4GHz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BandMode {
    /// 2.4 GHz only (default, supported by all ESP32 variants).
    #[default]
    Band2_4G,
    /// 5 GHz only (ESP32-C5 only).
    Band5G,
    /// Dual-band 2.4 GHz + 5 GHz (ESP32-C5 only).
    Auto,
}

/// `WiFi` access point configuration.
///
/// Contains settings for running the device as a `WiFi` access point.
#[derive(Clone, Debug)]
pub struct WifiApConfigStatic {
    /// Wifi Mode - Access Point (ap) or Station (sta) Mode. Access Point by default.
    /// Network name (SSID), max 32 characters.
    pub ap_ssid: String<32>,
    pub sta_ssid: String<32>,
    /// Mandatory `WiFi` password, max 63 characters.
    /// We don't want None here as it would present an open network,
    /// which is not something we want to support.
    pub ap_password: String<63>,
    pub sta_password: String<63>,
    /// `WiFi` channel (1-14 for 2.4GHz, 36+ for 5GHz).
    pub channel: u8,
    /// `WiFi` band mode (2.4GHz / 5GHz / Auto). Ignored on chips without 5GHz.
    pub band: BandMode,
    /// MAC address for the access point interface.
    pub mac: [u8; 6],
}

impl Default for WifiApConfigStatic {
    fn default() -> Self {
        Self {
            ap_ssid: String::new(),
            ap_password: String::new(),
            sta_ssid: String::new(),
            sta_password: String::new(),
            channel: 1,
            band: BandMode::default(),
            mac: [0; 6],
        }
    }
}
