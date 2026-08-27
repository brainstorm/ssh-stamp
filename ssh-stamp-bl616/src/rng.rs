// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Randomness, from the BL616's hardware TRNG.
//!
//! There is no entropy handover to arrange here, unlike the ESP port: the
//! TRNG lives in the security engine and is available for the whole lifetime
//! of the firmware, so boot-time key minting and per-connection SSH key
//! exchange draw from the same source.
//!
//! A failure is an error rather than a fallback. Anything weaker than the
//! TRNG is not an acceptable substitute for material that ends up in a host
//! key.

use ssh_stamp_hal::{HalError, RngHal};

/// Fill `buf` from the hardware generator.
///
/// # Errors
///
/// [`HalError::Rng`] if the security engine reports a failure.
pub fn fill_bytes(buf: &mut [u8]) -> Result<(), HalError> {
    bl616_wifi::rng::fill(buf).map_err(|_| HalError::Rng)
}

/// A random `u64`, for stack seeds.
///
/// # Errors
///
/// As [`fill_bytes`].
pub fn u64() -> Result<u64, HalError> {
    bl616_wifi::rng::u64().map_err(|_| HalError::Rng)
}

/// The HAL's RNG.
pub struct Bl616Rng;

impl RngHal for Bl616Rng {
    async fn fill_bytes(&mut self, buf: &mut [u8]) -> Result<(), HalError> {
        fill_bytes(buf)
    }
}
