// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! RNG for the RP2350, backed by the on-chip TRNG.
//!
//! `ssh-stamp` mints the SSH host key and the first-boot secrets through
//! `getrandom`, so a real entropy source must be registered before the
//! config is loaded — same contract as the ESP port, which goes out of its
//! way to enable true RNG for exactly this reason.
//!
//! We use the RP2350's TRNG block (ring oscillator sampled and
//! post-processed in hardware; `embassy-rp` marks it `CryptoRng`) rather
//! than the plain `RoscRng` counter, which is not an entropy source
//! suitable for key material.

use core::cell::RefCell;

use embassy_rp::peripherals::TRNG;
use embassy_rp::trng::Trng;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use getrandom::register_custom_getrandom;
use ssh_stamp_hal::{HalError, RngHal};

/// The registered TRNG. `'static` because it must outlive every consumer.
type SharedTrng = Trng<'static, TRNG>;

static RNG: Mutex<CriticalSectionRawMutex, RefCell<Option<SharedTrng>>> =
    Mutex::new(RefCell::new(None));

register_custom_getrandom!(rp2350_getrandom_custom_func);

/// Register the hardware TRNG for use with `getrandom`. Call once, before
/// any key material is generated.
pub fn register_custom_rng(trng: SharedTrng) {
    RNG.lock(|slot| slot.borrow_mut().replace(trng));
}

/// Fill `buf` from the registered TRNG.
///
/// # Errors
///
/// Returns [`HalError::Rng`] if [`register_custom_rng`] has not run yet.
pub fn fill_bytes(buf: &mut [u8]) -> Result<(), HalError> {
    RNG.lock(|slot| {
        let mut slot = slot.borrow_mut();
        let trng = slot.as_mut().ok_or(HalError::Rng)?;
        trng.blocking_fill_bytes(buf);
        Ok(())
    })
}

/// RP2350 RNG handle.
pub struct Rp2350Rng;

impl Rp2350Rng {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for Rp2350Rng {
    fn default() -> Self {
        Self::new()
    }
}

impl RngHal for Rp2350Rng {
    async fn fill_bytes(&mut self, buf: &mut [u8]) -> Result<(), HalError> {
        fill_bytes(buf)
    }
}

/// `getrandom` backend.
///
/// # Errors
///
/// Returns an error if [`register_custom_rng`] has not been called yet.
pub fn rp2350_getrandom_custom_func(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    RNG.lock(|slot| {
        let mut slot = slot.borrow_mut();
        let trng = slot.as_mut().ok_or(getrandom::Error::UNEXPECTED)?;
        trng.blocking_fill_bytes(buf);
        Ok(())
    })
}
