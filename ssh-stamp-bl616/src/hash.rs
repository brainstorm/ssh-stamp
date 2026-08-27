// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! SHA-256 and HMAC-SHA-256.
//!
//! Software, from `bl616-crypto`, which is the same `RustCrypto` generation
//! sunset already links -- so these share code with the SSH transport rather
//! than adding a second implementation. The BL616 does have a hardware
//! accelerator in its security engine; it is not used here because the
//! shared-code saving is worth more than the cycles until something profiles
//! otherwise.

use ssh_stamp_hal::{HalError, HashHal};

/// The HAL's hasher.
pub struct Bl616Hash;

impl HashHal for Bl616Hash {
    async fn sha256(&mut self, message: &[u8], output: &mut [u8; 32]) -> Result<(), HalError> {
        *output = bl616_crypto::hash::sha256(message);
        Ok(())
    }

    async fn hmac_sha256(
        &mut self,
        key: &[u8],
        message: &[u8],
        output: &mut [u8; 32],
    ) -> Result<(), HalError> {
        *output = bl616_crypto::hash::hmac_sha256(key, message);
        Ok(())
    }
}
