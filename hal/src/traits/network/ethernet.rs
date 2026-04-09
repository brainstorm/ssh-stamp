// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Ethernet trait for wired networking.

use core::future::Future;

use crate::{EthernetConfig, HalError};

/// Ethernet hardware abstraction.
///
/// Provides configuration and control of Ethernet PHY/MAC.
/// Implementations manage the underlying Ethernet controller.
///
/// # Example
///
/// ```ignore
/// async fn setup_ethernet<E: EthernetHal>(eth: &mut E) -> Result<(), HalError> {
///     let config = EthernetConfig {
///         mac: [0x02, 0x03, 0x04, 0x05, 0x06, 0x07],
///     };
///     eth.init(config).await
/// }
/// ```
pub trait EthernetHal {
    /// Initialize Ethernet with given configuration.
    ///
    /// Configures the Ethernet peripheral with the specified MAC address
    /// and brings up the network interface.
    ///
    /// # Arguments
    ///
    /// * `config` - Ethernet configuration (primarily MAC address).
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or a [`HalError`] error on failure.
    fn init(&mut self, config: EthernetConfig) -> impl Future<Output = Result<(), HalError>>;
}
