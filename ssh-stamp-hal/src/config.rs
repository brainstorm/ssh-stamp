// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Hardware configuration types.

use core::str::FromStr;
use heapless::String;

/// UART parity bit setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Parity {
    /// No parity bit (default).
    #[default]
    None,
    /// Even parity.
    Even,
    /// Odd parity.
    Odd,
}

impl FromStr for Parity {
    type Err = ();

    /// Parses a parity setting from a string value.
    ///
    /// Accepts `"none"`/`"n"`, `"even"`/`"e"` or `"odd"`/`"o"` (case-insensitive).
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("n") {
            Ok(Self::None)
        } else if value.eq_ignore_ascii_case("even") || value.eq_ignore_ascii_case("e") {
            Ok(Self::Even)
        } else if value.eq_ignore_ascii_case("odd") || value.eq_ignore_ascii_case("o") {
            Ok(Self::Odd)
        } else {
            Err(())
        }
    }
}

impl From<u8> for Parity {
    /// Resolves a `Parity` from its on-wire `u8` representation.
    ///
    /// Unknown values fall back to `None` (the default).
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Even,
            2 => Self::Odd,
            _ => Self::None,
        }
    }
}

/// UART line parameters for the SSH-to-serial bridge.
///
/// Persisted in the device config and applied when the bridge's UART is
/// brought up, so changes take effect on the next boot. Values are kept
/// target-agnostic; each port maps them onto its own UART driver types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UartParams {
    /// Baud rate in bits per second.
    pub baud: u32,
    /// Data bits per frame (5-8).
    pub data_bits: u8,
    /// Parity bit setting.
    pub parity: Parity,
    /// Stop bits per frame (1 or 2).
    pub stop_bits: u8,
}

impl Default for UartParams {
    /// The classic 115200 8N1.
    fn default() -> Self {
        Self {
            baud: 115_200,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
        }
    }
}

/// UART peripheral configuration.
///
/// Pin numbers (`tx_pin`, `rx_pin`) are target-specific and must be set by
/// the port binary before use. There are no cross-platform default values;
/// each port crate defines pin assignments in its `src/bin/` entry point.
/// See the `ssh-stamp-esp32` binary's module documentation for ESP32 defaults.
#[derive(Clone, Debug, Default)]
pub struct UartConfig {
    pub tx_pin: u8,
    pub rx_pin: u8,
    pub cts_pin: Option<u8>,
    pub rts_pin: Option<u8>,
    pub params: UartParams,
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

impl FromStr for BandMode {
    type Err = ();

    /// Parses a `WiFi` band mode from a string value.
    ///
    /// Accepts `"2.4g"`, `"2g"`, `"24g"`, `"5g"`, or `"auto"` (case-insensitive).
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("2.4g")
            || value.eq_ignore_ascii_case("2g")
            || value.eq_ignore_ascii_case("24g")
        {
            Ok(Self::Band2_4G)
        } else if value.eq_ignore_ascii_case("5g") {
            Ok(Self::Band5G)
        } else if value.eq_ignore_ascii_case("auto") {
            Ok(Self::Auto)
        } else {
            Err(())
        }
    }
}

impl From<u8> for BandMode {
    /// Resolves a `BandMode` from its on-wire `u8` representation.
    ///
    /// Unknown values fall back to `Band2_4G` (the default).
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Band5G,
            2 => Self::Auto,
            _ => Self::Band2_4G,
        }
    }
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
