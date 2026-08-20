// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! OTA update traits.
//!
//! Flash storage operations should use `embedded_storage_async::nor_flash::NorFlash`
//! from the embedded-hal ecosystem rather than a custom trait.
//!
//! OTA update actions are kept here because they are application-specific
//! (partition management, firmware validation) and not covered by embedded-hal.

use core::future::Future;

use crate::HalError;

/// OTA update operations.
///
/// # Errors
///
/// All methods return `HalError` on failure.
pub trait OtaActions {
    /// Identifier of the chip this firmware was built for, e.g. `"esp32c6"`.
    ///
    /// An incoming OTA image may carry the chip it was packed for; if the two
    /// disagree the transfer is refused before any of it reaches flash. Use
    /// the same spelling as the port's build tooling (`esp_hal::chip!()` on
    /// Espressif parts) so a `packer --target` invocation is predictable.
    const TARGET_CHIP: &'static str;

    /// Validate the current OTA partition.
    fn try_validating_current_ota_partition() -> impl Future<Output = Result<(), HalError>> + Send;

    /// Get size of OTA partition in bytes.
    fn get_ota_partition_size() -> impl Future<Output = Result<u32, HalError>> + Send;

    /// Write data to OTA partition at offset.
    fn write_ota_data(
        &self,
        offset: u32,
        data: &[u8],
    ) -> impl Future<Output = Result<(), HalError>> + Send;

    /// Finalize OTA update and mark for boot.
    fn finalize_ota_update(&mut self) -> impl Future<Output = Result<(), HalError>> + Send;

    /// Reset device to boot into new partition.
    fn reset_device(&self) -> !;
}
