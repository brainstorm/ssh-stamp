// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! HMAC-SHA256 implementation for ESP32 family
//!
//! Uses ESP32's hardware-accelerated HMAC peripheral.

use hal::{HashError, HashHal};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256 as Sha256Impl};

/// ESP32 HMAC implementation  
pub struct EspHmac;

impl HashHal for EspHmac {
    async fn hmac_sha256(
        &mut self,
        key: &[u8],
        message: &[u8],
        output: &mut [u8; 32],
    ) -> Result<(), hal::HalError> {
        // Use software HMAC implementation for now
        // ESP32 hardware HMAC requires special key handling
        let mut mac = Hmac::<Sha256Impl>::new_from_slice(key)
            .map_err(|_| hal::HalError::Hash(HashError::Config))?;
        mac.update(message);
        let result = mac.finalize();
        output.copy_from_slice(&result.into_bytes());
        Ok(())
    }

    async fn sha256(&mut self, message: &[u8], output: &mut [u8; 32]) -> Result<(), hal::HalError> {
        let mut hasher = Sha256Impl::new();
        hasher.update(message);
        let result = hasher.finalize();
        output.copy_from_slice(&result);
        Ok(())
    }
}
