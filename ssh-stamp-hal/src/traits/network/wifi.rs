// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 gabriel.ku <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `WiFi` hardware abstraction trait.

use core::future::Future;

use crate::{HalError, WifiApConfigStatic};

/// `WiFi` hardware abstraction.
///
/// Provides configuration and control of `WiFi` hardware.
/// Implementations manage the underlying `WiFi` radio and `TCP/IP` stack.
///
/// Currently supports access point (AP) mode. Station (STA) mode will be
/// added when client connectivity to existing networks is needed.
///
/// # Example
///
/// ```ignore
/// async fn start_wifi_ap<W: WifiHal>(wifi: &mut W) -> Result<(), HalError> {
///     let config = WifiApConfigStatic {
///         ssid: heapless::String::from("MyDevice"),
///         password: Some(heapless::String::from("secretpass")),
///         channel: 6,
///         mac: [0x02, 0x03, 0x04, 0x05, 0x06, 0x07],
///     };
///     wifi.start_ap(config).await
/// }
/// ```
pub trait WifiHal {
    /// Start `WiFi` access point with given configuration.
    ///
    /// Initializes the `WiFi` radio and starts broadcasting an access point
    /// with the specified `SSID`, password, and channel.
    ///
    /// # Arguments
    ///
    /// * `config` - `AP` configuration including `SSID`, password, channel, and `MAC`.
    ///
    /// # Errors
    ///
    /// Returns `HalError::Wifi` on failure.
    fn start_ap(
        &mut self,
        config: WifiApConfigStatic,
    ) -> impl Future<Output = Result<(), HalError>>;
}
