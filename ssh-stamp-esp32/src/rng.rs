// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! RNG implementation for ESP32 family
//!
//! Provides hardware random number generation using ESP32's true RNG.

use core::cell::RefCell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use esp_hal::rng::Rng;
use getrandom::register_custom_getrandom;
use ssh_stamp_hal::{HalError, RngHal};
use static_cell::StaticCell;

static RNG: StaticCell<Rng> = StaticCell::new();
static RNG_MUTEX: Mutex<CriticalSectionRawMutex, RefCell<Option<&'static mut Rng>>> =
    Mutex::new(RefCell::new(None));

register_custom_getrandom!(esp_getrandom_custom_func);

/// Register the hardware RNG for use with getrandom
pub fn register_custom_rng(rng: Rng) {
    let rng_ref = RNG.init(rng);
    RNG_MUTEX.lock(|t| t.borrow_mut().replace(rng_ref));
}

/// ESP32 RNG implementation
pub struct EspRng;

impl EspRng {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for EspRng {
    fn default() -> Self {
        Self::new()
    }
}

impl RngHal for EspRng {
    async fn fill_bytes(&mut self, buf: &mut [u8]) -> Result<(), HalError> {
        RNG_MUTEX.lock(|t| {
            let mut rng = t.borrow_mut();
            let rng = rng.as_mut().ok_or(HalError::Rng)?;
            rng.read(buf);
            Ok(())
        })
    }
}

/// Custom getrandom function for ESP-HAL.
///
/// # Errors
///
/// Returns an error if the RNG has not been registered via `register_custom_rng`.
///
/// # Panics
///
/// Panics if the RNG mutex lock fails internally.
pub fn esp_getrandom_custom_func(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    RNG_MUTEX.lock(|t| {
        let mut rng_ref = t.borrow_mut();
        let rng = rng_ref.as_mut().ok_or(getrandom::Error::UNEXPECTED)?;
        rng.read(buf);
        Ok(())
    })
}
