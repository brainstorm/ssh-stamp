// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::future::Future;

use crate::HalError;

/// Hash/HMAC operations
pub trait HashHal {
    /// Compute HMAC-SHA256
    fn hmac_sha256(
        &mut self,
        key: &[u8],
        message: &[u8],
        output: &mut [u8; 32],
    ) -> impl Future<Output = Result<(), HalError>>;

    /// Compute SHA256
    fn sha256(
        &mut self,
        message: &[u8],
        output: &mut [u8; 32],
    ) -> impl Future<Output = Result<(), HalError>>;
}
