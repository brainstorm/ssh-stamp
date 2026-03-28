// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use hal::{HardwareConfig, UartConfig, WifiApConfigStatic};
use heapless::String;

/// Default peripheral configuration for ESP32-C6
#[cfg(feature = "esp32c6")]
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
            ssid: String::try_from("ssh-stamp").unwrap_or_default(),
            password: None,
            channel: 1,
            mac: [0; 6], // Will be set from eFuse
        },
    }
}

/// Default peripheral configuration for ESP32-S3
#[cfg(feature = "esp32s3")]
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
            ssid: String::try_from("ssh-stamp").unwrap_or_default(),
            password: None,
            channel: 1,
            mac: [0; 6],
        },
    }
}

/// Default peripheral configuration for ESP32
#[cfg(feature = "esp32")]
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
            ssid: String::try_from("ssh-stamp").unwrap_or_default(),
            password: None,
            channel: 1,
            mac: [0; 6],
        },
    }
}

/// Default peripheral configuration for ESP32-S2
#[cfg(feature = "esp32s2")]
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
            ssid: String::try_from("ssh-stamp").unwrap_or_default(),
            password: None,
            channel: 1,
            mac: [0; 6],
        },
    }
}

/// Default peripheral configuration for ESP32-C3
#[cfg(feature = "esp32c3")]
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
            ssid: String::try_from("ssh-stamp").unwrap_or_default(),
            password: None,
            channel: 1,
            mac: [0; 6],
        },
    }
}

/// Default peripheral configuration for ESP32-C2
#[cfg(feature = "esp32c2")]
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
            ssid: String::try_from("ssh-stamp").unwrap_or_default(),
            password: None,
            channel: 1,
            mac: [0; 6],
        },
    }
}
