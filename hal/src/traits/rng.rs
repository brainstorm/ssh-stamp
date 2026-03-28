// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Random number generation trait.

use core::future::Future;

use crate::HalError;

/// Random number generation hardware abstraction.
///
/// Provides cryptographically secure random number generation.
/// Implementations typically wrap hardware TRNG peripherals.
///
/// # Example
///
/// ```ignore
/// async fn generate_nonce<R: RngHal>(rng: &mut R) -> Result<[u8; 16], HalError> {
///     let mut buf = [0u8; 16];
///     rng.fill_bytes(&mut buf).await?;
///     Ok(buf)
/// }
/// ```
pub trait RngHal {
    /// Fill buffer with random bytes.
    ///
    /// Generates cryptographically secure random bytes and fills the buffer.
    ///
    /// # Arguments
    ///
    /// * `buf` - Buffer to fill with random bytes.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or an error if generation fails.
    fn fill_bytes(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<(), HalError>>;

    /// Generate a random u32.
    ///
    /// Convenience method that generates 4 random bytes and converts to u32.
    ///
    /// # Returns
    ///
    /// Random u32 value on success, or an error if generation fails.
    fn random_u32(&mut self) -> impl Future<Output = Result<u32, HalError>> {
        async {
            let mut buf = [0u8; 4];
            self.fill_bytes(&mut buf).await?;
            Ok(u32::from_le_bytes(buf))
        }
    }
}