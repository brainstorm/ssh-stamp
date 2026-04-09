// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Network peripheral traits.

mod ethernet;
mod wifi;

pub use ethernet::EthernetHal;
pub use wifi::WifiHal;
