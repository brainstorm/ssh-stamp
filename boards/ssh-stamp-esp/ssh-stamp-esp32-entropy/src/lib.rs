// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The entropy source for ssh-stamp on ESP32 chips. This is setup as a separate
//! crate to ensure that randomness is called safely in all instances.
//!
//! # The entropy source
//!
//! The RNG register only produces true random numbers while an entropy source
//! is active. The two sources used must not overlap:
//!
//! 1. The SAR ADC source, which [`BootEntropy::start`] switches on early in
//!    boot. This covers the host key and `WiFi` PSK on first boot.
//! 2. The RF subsystem, which becomes a source once the radio is started, and
//!    covers the key exchange of every SSH connection after that.
//!
//! Espressif requires the ADC source to be switched off "before RF subsystem
//! features, ADC, or I2S (ESP32 only) are initialized", as it is "not safe to
//! use if any other subsystem is accessing the RF subsystem or the ADC at the
//! same time". [`BootEntropy::hand_over_to_radio`] switches from one to the
//! other.
//!
//! See: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/random.html>
//!
//! # Failing the ordering
//!
//! [`fill`] refuses to produce random numbers while a source is not active, and
//! returns [`NO_ENTROPY_SOURCE`] instead. A missed or misordered entropy
//! therefore shows up as an error can cannot produce insecure keys.
//!
//! # The `getrandom` backend
//!
//! getrandom picks its backend per target, and
//! `--cfg getrandom_backend="extern_impl"` selects it using a safe
//! function, which must be defined once for the whole program. The backend
//! is built on the unstable `extern_item_impls` feature, which is
//! why the firmware needs a nightly toolchain. In exchange, there is no
//! `unsafe` code in ssh-stamp.
//!
//! # ESP32-C5 and ESP32-C61
//!
//! esp-hal has no TRNG driver for these chips, so there is no boot-time
//! source to switch on and nothing tracks whether one is active. This means
//! that random numbers before the radio are as good as the RNG register,
//! and [`fill`] won't refuse.

#![no_std]
#![forbid(unsafe_code)]
#![allow(
    clippy::disallowed_types,
    reason = "this is the one crate that may read the hardware RNG, see clippy.toml"
)]

use core::mem::MaybeUninit;

use esp_hal::peripherals::{ADC1, RNG};
use zeroize::Zeroizing;

/// The error [`fill`] returns when no entropy source is active.
pub const NO_ENTROPY_SOURCE: getrandom::Error = getrandom::Error::new_custom(0);

/// The entropy source.
///
/// Having this keeps the ADC entropy source switched on, from
/// [`BootEntropy::start`] until [`BootEntropy::hand_over_to_radio`]. Dropping
/// it switches the source off without a replacement, after which [`fill`]
/// will fail until the radio is up.
#[must_use = "dropping switches entropy source off"]
pub struct BootEntropy {
    _source: source::Source,
}

impl BootEntropy {
    /// Switches the ADC entropy source on.
    ///
    /// From here until [`BootEntropy::hand_over_to_radio`], [`fill`] and
    /// `getrandom` produce true random numbers.
    pub fn start(rng: RNG<'static>, adc: ADC1<'static>) -> Self {
        Self {
            _source: source::enable(rng, adc),
        }
    }

    /// Switches the ADC source off, then calls `start_radio`, which ensures
    /// that the RF subsystem is started.
    ///
    /// A source is not active while `start_radio` runs, so [`fill`] will fail inside
    /// it. Once this returns the radio is expected to be on. If it fails [`fill`],
    /// will keep failing.
    pub fn hand_over_to_radio<R>(self, start_radio: impl FnOnce() -> R) -> R {
        drop(self);
        let started = start_radio();

        if !is_active() {
            log::error!("RNG unavailable, the radio did not take over as the entropy source");
        }

        started
    }
}

/// Whether an entropy source is active.
///
/// Always true on the ESP32-C5/C61.
#[must_use]
pub fn is_active() -> bool {
    source::is_active()
}

/// Fills `buf` from the hardware RNG.
///
/// This is what `getrandom` calls, so every random number in the firmware
/// will come through this function.
///
/// # Errors
///
/// Returns [`NO_ENTROPY_SOURCE`] while no entropy source is active.
pub fn fill(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    if source::read(buf) {
        Ok(())
    } else {
        log::error!("Refusing to produce random numbers, no entropy source is active");
        Err(NO_ENTROPY_SOURCE)
    }
}

/// The `getrandom` backend.
#[getrandom::implementation::fill_uninit]
fn fill_uninit(dest: &mut [MaybeUninit<u8>]) -> Result<(), getrandom::Error> {
    let mut block = Zeroizing::new([0u8; 64]);
    for chunk in dest.chunks_mut(block.len()) {
        let random = &mut block[..chunk.len()];
        fill(random)?;
        chunk.write_copy_of_slice(random);
    }
    Ok(())
}

/// Chips with a TRNG driver in esp-hal.
#[cfg(not(any(feature = "esp32c5", feature = "esp32c61")))]
mod source {
    use esp_hal::peripherals::{ADC1, RNG};
    use esp_hal::rng::{Trng, TrngSource};

    /// The SAR ADC entropy source, turned off when dropped.
    pub(crate) struct Source {
        _source: TrngSource<'static>,
    }

    pub(crate) fn enable(rng: RNG<'static>, adc: ADC1<'static>) -> Source {
        Source {
            _source: TrngSource::new(rng, adc),
        }
    }

    /// esp-hal counts the active sources, i.e. the ADC one, and the radio once
    /// esp-radio has started it.
    pub(crate) fn is_active() -> bool {
        TrngSource::is_enabled()
    }

    /// This reads through a [`Trng`], which esp-hal only gives while a source
    /// is active.
    pub(crate) fn read(buf: &mut [u8]) -> bool {
        let Ok(trng) = Trng::try_new() else {
            return false;
        };
        trng.read(buf);
        true
    }
}

// TODO: esp-hal reports `rng.trng_supported = false` for the ESP32-C5 and
// C61, so `Trng`/`TrngSource` are not compiled for those chips and this
// module is defined for them.
/// Chips without a TRNG driver in esp-hal.
#[cfg(any(feature = "esp32c5", feature = "esp32c61"))]
mod source {
    use esp_hal::peripherals::{ADC1, RNG};
    use esp_hal::rng::Rng;

    /// Holds the peripherals a TRNG source would take, so that the API is
    /// the same as on the other chips.
    pub(crate) struct Source {
        _rng: RNG<'static>,
        _adc: ADC1<'static>,
    }

    pub(crate) fn enable(rng: RNG<'static>, adc: ADC1<'static>) -> Source {
        log::warn!(
            "No TRNG on this chip, RNG is not cryptographically secure until the radio is up"
        );
        Source {
            _rng: rng,
            _adc: adc,
        }
    }

    /// Nothing tracks the sources on this chip.
    pub(crate) fn is_active() -> bool {
        true
    }

    pub(crate) fn read(buf: &mut [u8]) -> bool {
        Rng::new().read(buf);
        true
    }
}
