// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
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
//! Defining that symbol requires `unsafe`, which this crate forbids, and
//! binaries cannot link each other's definitions. So, the
//! [`getrandom_backend!`](macro@crate::getrandom_backend) packages the
//! definition as a macro that every binary invokes once. The `unsafe`
//! only compiles where the macro is expanded, keeping this crate
//! `#![forbid(unsafe_code)]`.

use core::cell::RefCell;
use core::future::{Future, ready};

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use esp_hal::rng::Rng;
#[cfg(not(any(feature = "esp32c5", feature = "esp32c61")))]
use esp_hal::{
    peripherals::{ADC1, RNG},
    rng::{Trng, TrngSource},
};
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

/// This is a wrapper that sets up the boot entropy source.
///
/// On chips with a TRNG this holds the SAR ADC entropy source that
/// [`init_entropy()`] enables. The RNG register only has randomness
/// while a source is active, and the ADC source is what occurs on boot.
///
/// Dropping this guard switches off the source. This handover is required
/// because the ADC source needs to be switched off "before RF subsystem
/// features, ADC, or I2S (ESP32 only) are initialized" and that it's "not
/// safe to use if any other subsystem is accessing the RF subsystem or
/// the ADC at the same time".
///
/// On the ESP32-C5/C61, there is no TRNG driver yet, so the guard is empty.
///
/// See: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/random.html>
#[must_use = "dropping switches the boot-time entropy source off"]
pub struct EntropySource {
    #[cfg(not(any(feature = "esp32c5", feature = "esp32c61")))]
    _source: TrngSource<'static>,
}

// TODO: The ESP32-C5/C61 TRNG is not yet available in esp-hal 1.1.1. Once
// https://github.com/esp-rs/esp-hal/pull/4978 lands in a release, remove
// this cfg_if! and use Trng/TrngSource unconditionally for all targets.
cfg_if::cfg_if! {
    if #[cfg(any(feature = "esp32c5", feature = "esp32c61"))] {
        /// Registers the hardware RNG with `getrandom`.
        ///
        /// The ESP32-C5/C61 have no TRNG in esp-hal yet, so the RNG register
        /// is all the firmware has at boot. This means that anything using
        /// this before the radio is up is only as good as that register.
        ///
        /// Call through [`init_entropy!`](macro@crate::init_entropy), which moves =
        /// the peripherals needed.
        pub fn init_entropy() -> (Rng, EntropySource) {
            log::warn!(
                "No TRNG on this chip, RNG is not cryptographically secure until the radio is up"
            );

            let rng = Rng::new();
            register_custom_rng(rng);

            (rng, EntropySource {})
        }

        /// Whether an entropy source is currently using the RNG register.
        /// Always true for esp32c5/c61 as there is no driver.
        #[must_use]
        pub fn entropy_source_active() -> bool {
            true
        }
    } else {
        /// Enables true random number generation and registers the hardware
        /// RNG with `getrandom`, which is used by the SSH host key and
        /// the `WiFi` password.
        ///
        /// The RNG register only produces true random numbers while an
        /// entropy source is active. These are the SAR ADC source enabled here,
        /// which covers early boot, and the RF system, which covers once `WiFi`
        /// is up. The returned [`Rng`] doesn't have this information, it just
        /// uses whichever source is active at the time to decide the quality.
        ///
        /// This means that the [`EntropySource`] must be kept alive until the radio
        /// takes over, and then it should be dropped.
        ///
        /// Call through [`init_entropy!`](macro@crate::init_entropy), which moves
        /// the peripherals the active chip needs in.
        ///
        /// See: <https://github.com/brainstorm/ssh-stamp/issues/10>
        /// See: <https://github.com/esp-rs/esp-hal/pull/3829>
        ///
        /// # Panics
        ///
        /// Panics if the TRNG is unavailable, which cannot happen as
        /// the source is created first.
        pub fn init_entropy(
            rng: RNG<'static>,
            adc: ADC1<'static>,
        ) -> (Rng, EntropySource) {
            let source = TrngSource::new(rng, adc);
            let trng = Trng::try_new().expect("TrngSource was just created");
            let handle = trng.downgrade();

            register_custom_rng(handle);

            (handle, EntropySource { _source: source })
        }

        /// Whether an entropy source is currently using the RNG register.
        ///
        /// Used to guard call sites with `debug_assert!`.
        #[must_use]
        pub fn entropy_source_active() -> bool {
            TrngSource::is_enabled()
        }
    }
}

/// Creates the boot entropy source and registers the hardware RNG with
/// `getrandom`.
///
/// This is a convenience macro that allows creating the source without having
/// to specify the features manually.
#[macro_export]
macro_rules! init_entropy {
    ($peripherals:expr) => {{
        #[cfg(any(feature = "esp32c5", feature = "esp32c61"))]
        let out = $crate::init_entropy();
        #[cfg(not(any(feature = "esp32c5", feature = "esp32c61")))]
        let out = $crate::init_entropy($peripherals.RNG, $peripherals.ADC1);
        out
    }};
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
/// The `__getrandom_v03_custom` that  [`getrandom_backend!`](macro@crate::getrandom_backend)
/// defines forwards here. See the module docs for why the split exists.
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

/// Defines the `getrandom` custom backend, which is an `unsafe extern "Rust"` symbol
/// that fills a buffer from the hardware RNG registered by [`init_entropy!`](macro@crate::init_entropy).
/// This is what the SSH host key and the `WiFi` password use, so it must be
/// a true and secure source of randomness.
#[macro_export]
macro_rules! getrandom_backend {
    () => {
        #[unsafe(no_mangle)]
        unsafe extern "Rust" fn __getrandom_v03_custom(
            dest: *mut u8,
            len: usize,
        ) -> Result<(), $crate::getrandom::Error> {
            // SAFETY: getrandom guarantees `dest` is valid for writes of
            // `len` bytes. The buffer may be uninitialised, so it is zeroed
            // before a slice is formed over it, as getrandom's documentation
            // prescribes.
            let buf = unsafe {
                ::core::ptr::write_bytes(dest, 0, len);
                ::core::slice::from_raw_parts_mut(dest, len)
            };
            $crate::rng_fill_bytes(buf)
        }
    };
}
