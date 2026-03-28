// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::future::Future;

use crate::{HalError, WifiApConfigStatic};

/// WiFi hardware abstraction for access point mode
pub trait WifiHal {
    /// Start WiFi access point with given configuration
    fn start_ap(
        &mut self,
        config: WifiApConfigStatic,
    ) -> impl Future<Output = Result<(), HalError>>;
}
