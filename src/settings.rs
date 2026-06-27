// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Compile-time constants: default IP, `WiFi` character set, buffer sizes.

use core::net::Ipv4Addr;

// SSH server settings
//pub(crate) const MTU: usize = 1536;
//pub(crate) const PORT: u16 = 22;
pub(crate) const SSH_STAMP_IDENT: &str = env!("SSH_STAMP_IDENT");
pub(crate) const KEY_SLOTS: usize = 1; // TODO: Document whether this a "reasonable default"? Justify why?
pub const DEFAULT_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 4, 1); // TODO: Expose this setting via
// SSH_STAMP env var?

// WiFi SSID and password character set (alphanumeric)
pub(crate) const WIFI_PASSWORD_CHARS: &[u8; 62] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
// Wifi Station Mode Connection
pub const STATION_MODE_MAX_RETRY_SECONDS: u8 = 10;

// UART settings
pub const UART_BUFFER_SIZE: usize = 4096;
