// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::future::Future;

use crate::HalError;

/// Random number generation
pub trait RngHal {
    /// Fill buffer with random bytes
    fn fill_bytes(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<(), HalError>>;

    /// Generate a random u32
    fn random_u32(&mut self) -> impl Future<Output = Result<u32, HalError>> {
        async {
            let mut buf = [0u8; 4];
            self.fill_bytes(&mut buf).await?;
            Ok(u32::from_le_bytes(buf))
        }
    }
}
