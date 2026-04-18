// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `WiFi` implementation for ESP32 family
//!
//! Provides `WiFi` access point functionality for SSH-Stamp.

use embassy_net::tcp::TcpSocket;
use embassy_net::IpListenEndpoint;
use hal::{HalError, WifiApConfigStatic, WifiHal};
use log::debug;

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
    tcp_stack: embassy_net::Stack<'a>,
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
    {
    }
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