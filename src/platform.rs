// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Platform services abstraction.
//!
//! The HSM and SSH handlers call into the running platform for three
//! things that can't be expressed as a pure HAL trait (because they touch
//! app-layer state like [`SSHStampConfig`] or the serial bridge):
//!
//! * persisting the SSH-stamp config to non-volatile storage,
//! * resetting the device,
//! * minting an [`OtaActions`] writer for the SFTP OTA session,
//! * signalling the serial bridge that SSH is ready and the UART task
//!   should wake up.
//!
//! Each platform crate provides one impl (for ESP32: `EspPlatform`).
//! Consumers take `&impl PlatformServices` so the same app code runs on
//! every MCU port.

use core::future::Future;

use ssh_stamp_hal::{HalError, OtaActions};

use crate::config::SSHStampConfig;

/// Platform-owned services the app layer cannot provide on its own.
///
/// # Contract
///
/// * [`Self::save_config`] must be durable: after it returns `Ok(())` the
///   config must survive a reboot.
/// * [`Self::reset`] must not return.
/// * [`Self::ota_writer`] may be called multiple times; each call yields
///   a fresh writer suitable for a single OTA session.
/// * [`Self::activate_uart`] signals the platform's buffered UART task
///   (if any) that it is OK to start streaming. Idempotent.
pub trait PlatformServices {
    /// OTA writer type this platform provides. Must live for the whole
    /// SFTP session, so `'static` is required.
    type OtaWriter: OtaActions + 'static;

    /// Buffered CAN type this platform provides. The CAN pump task owns
    /// it for the lifetime of the device, so `'static` is required.
    #[cfg(feature = "can")]
    type Can: crate::can::BufferedCan + 'static;

    /// Access the platform's buffered CAN interface for the SSH `can`
    /// subsystem bridge.
    #[cfg(feature = "can")]
    fn can(&self) -> &'static Self::Can;

    /// Buffered I2C type this platform provides. The I2C pump task owns
    /// it for the lifetime of the device, so `'static` is required.
    #[cfg(feature = "i2c")]
    type I2c: crate::i2c::BufferedI2c + 'static;

    /// Access the platform's buffered I2C master for the SSH `i2c`
    /// subsystem bridge.
    #[cfg(feature = "i2c")]
    fn i2c(&self) -> &'static Self::I2c;

    /// Persist the full config to non-volatile storage.
    ///
    /// # Errors
    ///
    /// Returns `HalError::Flash` on write / erase failure.
    fn save_config(&self, config: &SSHStampConfig) -> impl Future<Output = Result<(), HalError>>;

    /// Reset the device. Does not return.
    fn reset(&self) -> !;

    /// Construct a fresh OTA writer for a new SFTP OTA session.
    fn ota_writer(&self) -> Self::OtaWriter;

    /// Signal the platform's buffered UART task that SSH is ready and
    /// UART transfer may start. Idempotent.
    fn activate_uart(&self);
}
