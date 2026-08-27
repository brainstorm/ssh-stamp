// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The handle through which the app layer reaches BL616-only services.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use ssh_stamp::config::SSHStampConfig;
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::store;
use ssh_stamp_hal::{FlashError, HalError, OtaActions};

use crate::flash::Bl616Flash;

/// Scratch for `store`, which needs somewhere to build a record before
/// writing it. One sector, matching the configuration area.
const CONFIG_BUF: usize = 4096;

/// Serialised access to the flash.
///
/// The SPI flash is also where the running firmware is executing from, so two
/// concurrent operations are not merely a data race over a buffer.
static FLASH: Mutex<CriticalSectionRawMutex, (Bl616Flash, [u8; CONFIG_BUF])> =
    Mutex::new((Bl616Flash, [0u8; CONFIG_BUF]));

/// BL616 platform services.
pub struct Bl616Platform;

impl Default for Bl616Platform {
    fn default() -> Self {
        Self::new()
    }
}

impl Bl616Platform {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl PlatformServices for Bl616Platform {
    type OtaWriter = Bl616OtaWriter;

    async fn save_config(&self, config: &SSHStampConfig) -> Result<(), HalError> {
        let mut guard = FLASH.lock().await;
        let (flash, buf) = &mut *guard;
        store::save(flash, buf, config).map_err(|_| HalError::Flash(FlashError::Write))
    }

    fn reset(&self) -> ! {
        bl616_wifi::runtime::reset()
    }

    fn ota_writer(&self) -> Self::OtaWriter {
        Bl616OtaWriter
    }

    fn activate_uart(&self) {
        crate::uart::UART_SIGNAL.signal(1);
    }
}

/// OTA is not implemented on this port.
///
/// The ESP path is built on `esp-bootloader-esp-idf`'s A/B partition scheme,
/// which has no counterpart here: the BL616 boot ROM reads a header this
/// project does not yet write a second copy of. Refusing is the honest
/// answer; a partial implementation that accepted an image and then bricked
/// the board on reset would be worse.
pub struct Bl616OtaWriter;

impl OtaActions for Bl616OtaWriter {
    async fn try_validating_current_ota_partition() -> Result<(), HalError> {
        Err(HalError::Flash(FlashError::InternalError))
    }

    async fn get_ota_partition_size() -> Result<u32, HalError> {
        Err(HalError::Flash(FlashError::InternalError))
    }

    async fn write_ota_data(&self, _offset: u32, _data: &[u8]) -> Result<(), HalError> {
        Err(HalError::Flash(FlashError::InternalError))
    }

    async fn finalize_ota_update(&mut self) -> Result<(), HalError> {
        Err(HalError::Flash(FlashError::InternalError))
    }

    fn reset_device(&self) -> ! {
        bl616_wifi::runtime::reset()
    }
}
