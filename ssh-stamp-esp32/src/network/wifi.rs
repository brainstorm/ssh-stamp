// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 gabriel.ku <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `WiFi` implementation for ESP32 family
//!
//! Provides `WiFi` access point functionality for SSH-Stamp.

use embassy_net::IpListenEndpoint;
use embassy_net::Stack;
use embassy_net::tcp::TcpSocket;
use log::debug;
use ssh_stamp_hal::{HalError, WifiApConfigStatic, WifiHal};

/// ESP32 `WiFi` implementation
pub struct EspWifi {
    ap_config: Option<WifiApConfigStatic>,
}

impl EspWifi {
    #[must_use]
    pub fn new() -> Self {
        Self { ap_config: None }
    }
}

impl Default for EspWifi {
    fn default() -> Self {
        Self::new()
    }
}

impl WifiHal for EspWifi {
    async fn start_ap(&mut self, config: WifiApConfigStatic) -> Result<(), HalError> {
        self.ap_config = Some(config);
        Ok(())
    }
}

/// Accept incoming TCP connection
pub async fn accept_requests<'a>(
    tcp_stack: Stack<'a>,
    rx_buffer: &'a mut [u8],
    tx_buffer: &'a mut [u8],
) -> TcpSocket<'a> {
    let mut tcp_socket = TcpSocket::new(tcp_stack, rx_buffer, tx_buffer);

    debug!("Waiting for SSH client...");
    if let Err(_e) = tcp_socket
        .accept(IpListenEndpoint {
            addr: None,
            port: 22,
        })
        .await
    {}
    debug!("Connected, port 22");

    tcp_socket
}

/// Disable AP stack
pub async fn ap_stack_disable() {
    debug!("AP Stack disabled: WIP");
}

/// Disable TCP socket
pub async fn tcp_socket_disable() {
    debug!("TCP socket disabled: WIP");
}

/// Disable `WiFi` controller
pub async fn wifi_controller_disable() {
    debug!("Disabling wifi: WIP");
}
