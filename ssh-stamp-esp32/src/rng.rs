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
//!
//! # Wiring this into `getrandom`
//!
//! getrandom 0.4 no longer selects its backend with a cargo feature and no
//! longer offers `register_custom_getrandom!`. Instead the `custom` backend
//! is chosen per target with `--cfg getrandom_backend="custom"` (set in
//! `.cargo/config.toml` for every bare-metal target here), and getrandom
//! links an `extern "Rust"` symbol that must be defined exactly once in the
//! whole program.
//!
//! Defining that symbol requires `unsafe`, so it lives in the port binary
//! (`src/bin/ssh-stamp-esp32.rs`) — which is also where getrandom's own
//! documentation says it belongs — and simply forwards to [`fill_bytes`]
//! below. That keeps this crate `#![forbid(unsafe_code)]`.

use core::cell::RefCell;
use core::future::{Future, ready};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use esp_hal::rng::Rng;
use ssh_stamp_hal::{HalError, RngHal};
use static_cell::StaticCell;

static RNG: StaticCell<Rng> = StaticCell::new();
static RNG_MUTEX: Mutex<CriticalSectionRawMutex, RefCell<Option<&'static mut Rng>>> =
    Mutex::new(RefCell::new(None));

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
    fn fill_bytes(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<(), HalError>> {
        ready(RNG_MUTEX.lock(|t| {
            let mut rng = t.borrow_mut();
            let rng = rng.as_mut().ok_or(HalError::Rng)?;
            rng.read(buf);
            Ok(())
        }))
    }
}

/// Safe half of the `getrandom` custom backend: fills `buf` from the
/// registered hardware RNG.
///
/// The binary's `__getrandom_v03_custom` shim forwards here; see the module
/// docs for why the split exists.
///
/// # Errors
///
/// Returns an error if the RNG has not been registered via `register_custom_rng`.
///
/// # Panics
///
/// Panics if the RNG mutex lock fails internally.
pub fn fill_bytes(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    RNG_MUTEX.lock(|t| {
        let mut rng_ref = t.borrow_mut();
        let rng = rng_ref.as_mut().ok_or(getrandom::Error::UNEXPECTED)?;
        rng.read(buf);
        Ok(())
    })
}
