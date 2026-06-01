// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! ESP32 implementation of [`PlatformServices`].
//!
//! Wires the app layer's persistence, reset, OTA, and UART-activation hooks
//! through to ESP-specific helpers (`flash::*`, `esp_hal::system`, the
//! `UART_SIGNAL`).

use ssh_stamp::config::SSHStampConfig;
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::store;
use ssh_stamp_hal::{FlashError, HalError};

use crate::EspOtaWriter;
use crate::flash;
use crate::uart::UART_SIGNAL;

/// Zero-sized handle through which the app layer reaches ESP-only services.
///
/// Construct once on the embassy executor and pass `&EspPlatform` to
/// [`ssh_stamp::app::run_app`] / [`ssh_stamp::app::prepare_ap_config`].
pub struct EspPlatform;

impl EspPlatform {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for EspPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformServices for EspPlatform {
    type OtaWriter = EspOtaWriter;

    async fn save_config(&self, config: &SSHStampConfig) -> Result<(), HalError> {
        let Some(flash_guard) = flash::get_flash_n_buffer() else {
            return Err(HalError::Flash(FlashError::InternalError));
        };
        let mut fb = flash_guard.lock().await;
        let (flash, buf) = fb.split_ref_mut();
        store::save(flash, buf, config).map_err(|_| HalError::Flash(FlashError::Write))
    }

    fn reset(&self) -> ! {
        esp_hal::system::software_reset()
    }

    fn ota_writer(&self) -> Self::OtaWriter {
        EspOtaWriter::new()
    }

    fn activate_uart(&self) {
        UART_SIGNAL.signal(1);
    }
}
