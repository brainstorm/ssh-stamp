// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use heapless::String;
use ssh_stamp_hal::{HardwareConfig, UartConfig, WifiApConfigStatic};

/// Alphanumeric character set used for random `WiFi` SSID and password generation.
const WIFI_ALPHANUM_CHARS: &[u8; 62] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Generates a random `WiFi` SSID using cryptographically secure randomness.
///
/// # Panics
/// Panics if the RNG source is unavailable.
fn generate_wifi_ssid() -> String<32> {
    let mut rnd = [0u8; 16];
    getrandom::getrandom(&mut rnd).expect("RNG failed for SSID generation");
    let mut ssid = String::<32>::new();
    for &byte in &rnd {
        let _ = ssid.push(WIFI_ALPHANUM_CHARS[(byte as usize) % 62] as char);
    }
    ssid
}

/// Generates a random `WiFi` password using cryptographically secure randomness.
///
/// # Panics
/// Panics if the RNG source is unavailable.
fn generate_wifi_password() -> String<63> {
    let mut rnd = [0u8; 24];
    getrandom::getrandom(&mut rnd).expect("RNG failed for password generation");
    let mut pw = String::<63>::new();
    for &byte in &rnd {
        let _ = pw.push(WIFI_ALPHANUM_CHARS[(byte as usize) % 62] as char);
    }
    pw
}

/// Default peripheral configuration for ESP32-C6
#[cfg(feature = "esp32c6")]
#[must_use]
pub fn default_config() -> HardwareConfig {
    HardwareConfig {
        uart: UartConfig {
            tx_pin: 16,
            rx_pin: 17,
            cts_pin: Some(15),
            rts_pin: Some(18),
            baud_rate: 115_200,
        },
        wifi: WifiApConfigStatic {
            ssid: generate_wifi_ssid(),
            password: generate_wifi_password(),
            mac: [0; 6], // Will be set from eFuse
        },
    }
}

/// Default peripheral configuration for ESP32-S3
#[cfg(feature = "esp32s3")]
#[must_use]
pub fn default_config() -> HardwareConfig {
    HardwareConfig {
        uart: UartConfig {
            tx_pin: 43,
            rx_pin: 44,
            cts_pin: Some(45),
            rts_pin: Some(46),
            baud_rate: 115_200,
        },
        wifi: WifiApConfigStatic {
            ssid: generate_wifi_ssid(),
            password: generate_wifi_password(),
            mac: [0; 6],
        },
    }
}

/// Default peripheral configuration for ESP32
#[cfg(feature = "esp32")]
#[must_use]
pub fn default_config() -> HardwareConfig {
    HardwareConfig {
        uart: UartConfig {
            tx_pin: 4,
            rx_pin: 5,
            cts_pin: Some(6),
            rts_pin: Some(7),
            baud_rate: 115_200,
        },
        wifi: WifiApConfigStatic {
            ssid: generate_wifi_ssid(),
            password: generate_wifi_password(),
            mac: [0; 6],
        },
    }
}

/// Default peripheral configuration for ESP32-S2
#[cfg(feature = "esp32s2")]
#[must_use]
pub fn default_config() -> HardwareConfig {
    HardwareConfig {
        uart: UartConfig {
            tx_pin: 43,
            rx_pin: 44,
            cts_pin: None,
            rts_pin: None,
            baud_rate: 115_200,
        },
        wifi: WifiApConfigStatic {
            ssid: generate_wifi_ssid(),
            password: generate_wifi_password(),
            mac: [0; 6],
        },
    }
}

/// Default peripheral configuration for ESP32-C3
#[cfg(feature = "esp32c3")]
#[must_use]
pub fn default_config() -> HardwareConfig {
    HardwareConfig {
        uart: UartConfig {
            tx_pin: 2,
            rx_pin: 3,
            cts_pin: None,
            rts_pin: None,
            baud_rate: 115_200,
        },
        wifi: WifiApConfigStatic {
            ssid: generate_wifi_ssid(),
            password: generate_wifi_password(),
            mac: [0; 6],
        },
    }
}

/// Default peripheral configuration for ESP32-C2
#[cfg(feature = "esp32c2")]
#[must_use]
pub fn default_config() -> HardwareConfig {
    HardwareConfig {
        uart: UartConfig {
            tx_pin: 20,
            rx_pin: 21,
            cts_pin: None,
            rts_pin: None,
            baud_rate: 115_200,
        },
        wifi: WifiApConfigStatic {
            ssid: generate_wifi_ssid(),
            password: generate_wifi_password(),
            mac: [0; 6],
        },
    }
}
