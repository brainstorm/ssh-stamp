// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! RNG implementation for ESP32 family
//!
//! Provides hardware random number generation using ESP32's true RNG.

use core::cell::RefCell;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use esp_hal::rng::Rng;
use getrandom::register_custom_getrandom;
use hal::{HalError, RngHal};
use static_cell::StaticCell;

static RNG: StaticCell<Rng> = StaticCell::new();
static RNG_MUTEX: Mutex<CriticalSectionRawMutex, RefCell<Option<&'static mut Rng>>> =
    Mutex::new(RefCell::new(None));

/// ESP32 RNG implementation
pub struct EspRng {
    _inner: (),
}

impl EspRng {
    /// Create a new ESP RNG instance
    ///
    /// Note: The actual RNG must be registered via `register()` before use.
    pub fn new() -> Self {
        Self { _inner: () }
    }

    /// Register the RNG for use with getrandom
    pub fn register(rng: Rng) {
        let rng_ref = RNG.init(rng);
        RNG_MUTEX.lock(|t| t.borrow_mut().replace(rng_ref));
        register_custom_getrandom!(esp_getrandom_custom_func);
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

/// ESP32-specific getrandom implementation
pub fn esp_getrandom_custom_func(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    RNG_MUTEX.lock(|t| {
        let mut rng = t.borrow_mut();
        let rng = rng
            .as_mut()
            .expect("register() should have been called first");
        rng.read(buf);
    });
    Ok(())
}
