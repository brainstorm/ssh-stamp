// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! WiFi trait for access point mode.

use core::future::Future;

use crate::{HalError, WifiApConfigStatic};

/// WiFi hardware abstraction for access point mode.
///
/// Provides configuration and control of WiFi hardware in AP mode.
/// Implementations manage the underlying WiFi radio and TCP/IP stack.
///
/// # Example
///
/// ```ignore
/// async fn setup_wifi<W: WifiHal>(wifi: &mut W) -> Result<(), HalError> {
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
    /// Start WiFi access point with given configuration.
    ///
    /// Initializes the WiFi radio and starts broadcasting an access point
    /// with the specified SSID, password, and channel.
    ///
    /// # Arguments
    ///
    /// * `config` - AP configuration including SSID, password, channel, and MAC.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or a [`HalError::Wifi`] error on failure.
    fn start_ap(
        &mut self,
        config: WifiApConfigStatic,
    ) -> impl Future<Output = Result<(), HalError>>;
}
