// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Hash/HMAC operations trait.

use core::future::Future;

use crate::HalError;

/// Hash/HMAC hardware abstraction.
///
/// Provides cryptographic hash functions. Implementations may use
/// hardware accelerators or software implementations depending on
/// platform capabilities.
///
/// # Example
///
/// ```ignore
/// async fn compute_hash<H: HashHal>(hash: &mut H, data: &[u8]) -> Result<[u8; 32], HalError> {
///     let mut output = [0u8; 32];
///     hash.sha256(data, &mut output).await?;
///     Ok(output)
/// }
/// ```
pub trait HashHal {
    /// Compute HMAC-SHA256.
    ///
    /// Computes Hash-based Message Authentication Code using SHA256.
    /// Useful for message authentication and key derivation.
    ///
    /// # Arguments
    ///
    /// * `key` - Secret key for HMAC.
    /// * `message` - Data to authenticate.
    /// * `output` - Output buffer for 32-byte HMAC result.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success with result in `output`, or an error on failure.
    fn hmac_sha256(
        &mut self,
        key: &[u8],
        message: &[u8],
        output: &mut [u8; 32],
    ) -> impl Future<Output = Result<(), HalError>>;

    /// Compute SHA256.
    ///
    /// Computes the SHA256 hash of the input message.
    ///
    /// # Arguments
    ///
    /// * `message` - Data to hash.
    /// * `output` - Output buffer for 32-byte hash result.
    ///
    /// # Returns
    ///
    /// `Ok(())` on success with result in `output`, or an error on failure.
    fn sha256(
        &mut self,
        message: &[u8],
        output: &mut [u8; 32],
    ) -> impl Future<Output = Result<(), HalError>>;
}
