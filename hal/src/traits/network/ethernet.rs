// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::future::Future;

use crate::{EthernetConfig, HalError};

/// Ethernet hardware abstraction
pub trait EthernetHal {
    /// Initialize Ethernet with given configuration
    fn init(&mut self, config: EthernetConfig) -> impl Future<Output = Result<(), HalError>>;
}
