// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `WiFi` hardware abstraction trait.

use crate::{HalError, NetworkProviderHal, WifiApConfigStatic};

/// `WiFi`-specific extension of [`NetworkProviderHal`].
///
/// Platforms with a `WiFi` radio implement this in addition to
/// [`NetworkProviderHal`]. Consumers configure the access point (or, in
/// the future, station) parameters with [`Self::configure_ap`], then call
/// [`NetworkProviderHal::bring_up`] to start the stack.
///
/// Currently only access-point mode is defined. Station mode will be added
/// when client connectivity to existing networks is needed.
///
/// # Example
///
/// ```ignore
/// async fn start_ap<W: WifiHal>(wifi: &mut W) -> Result<(), HalError> {
///     let config = WifiApConfigStatic {
///         ssid: heapless::String::from("MyDevice"),
///         password: Some(heapless::String::from("secretpass")),
///         channel: 6,
///         mac: [0x02, 0x03, 0x04, 0x05, 0x06, 0x07],
///     };
///     wifi.configure_ap(config)?;
///     let _stack = wifi.bring_up().await?;
///     Ok(())
/// }
/// ```
pub trait WifiHal: NetworkProviderHal {
    /// Supply access-point parameters to apply at [`NetworkProviderHal::bring_up`] time.
    ///
    /// Must be called before `bring_up`. Implementations store the config
    /// and do not start the radio here; this keeps the trait
    /// synchronous and side-effect-free, which fits the "configure then
    /// bring up" pattern used by embassy-net drivers.
    ///
    /// # Errors
    ///
    /// Returns [`HalError::Wifi`] if the configuration is rejected by the
    /// driver (e.g. SSID too long after encoding, unsupported channel).
    fn configure_ap(&mut self, config: WifiApConfigStatic) -> Result<(), HalError>;
}
