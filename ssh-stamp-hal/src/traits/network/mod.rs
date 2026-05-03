// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Network-provider trait hierarchy.
//!
//! [`NetworkProviderHal`] is the generic base: any platform that can hand
//! back an [`embassy_net::Stack`] implements it, regardless of underlying
//! transport. [`WifiHal`] extends it with WiFi-specific configuration.
//!
//! Future transports (Ethernet PHY, USB-CDC-NCM, external modem) would add
//! their own extension traits alongside `WifiHal`.

mod provider;
mod wifi;

pub use provider::NetworkProviderHal;
pub use wifi::WifiHal;
