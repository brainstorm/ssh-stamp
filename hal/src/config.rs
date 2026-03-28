// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

#[derive(Clone, Debug)]
pub struct HardwareConfig {
    pub uart: UartConfig,
    pub wifi: WifiApConfigStatic,
}

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

#[derive(Clone, Debug)]
pub struct WifiApConfigStatic {
    pub ssid: heapless::String<32>,
    pub password: Option<heapless::String<63>>,
    pub channel: u8,
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

#[derive(Clone, Debug)]
pub struct EthernetConfig {
    pub mac: [u8; 6],
}

impl Default for EthernetConfig {
    fn default() -> Self {
        Self { mac: [0; 6] }
    }
}
