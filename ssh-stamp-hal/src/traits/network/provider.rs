// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Generic network-provider hardware abstraction.

use core::future::Future;

use embassy_net::Stack;

use crate::HalError;

/// A platform that can bring up a network interface.
///
/// This is the generic cut beneath [`crate::WifiHal`]: it says nothing about
/// what radio (if any) is attached and only promises to hand back an
/// [`embassy_net::Stack`] that is up and usable. Implementations may be
/// backed by built-in `WiFi`, an externally connected module (SDIO/SPI/USB
/// `WiFi`, ATWINC, cyw43, ESP-AT companion, CDC-NCM over USB), or a wired
/// Ethernet PHY.
///
/// Transport-specific configuration happens through the extended trait for
/// that transport (for `WiFi`, see [`crate::WifiHal::configure_ap`]). Call
/// those configuration methods first, then [`Self::bring_up`].
///
/// # Example
///
/// ```ignore
/// async fn start<N: NetworkProviderHal>(net: &mut N) -> Result<Stack<'static>, HalError> {
///     net.bring_up().await
/// }
/// ```
pub trait NetworkProviderHal {
    /// Bring the network interface up and wait for the link to become ready.
    ///
    /// Returns an [`embassy_net::Stack`] that is ready to open sockets on.
    /// Implementations typically spawn their driver tasks on the embassy
    /// executor during this call.
    ///
    /// # Errors
    ///
    /// Returns [`HalError::Wifi`] (or a future transport-specific variant)
    /// on link-up, DHCP, or driver-init failure.
    fn bring_up(&mut self) -> impl Future<Output = Result<Stack<'static>, HalError>>;
}
