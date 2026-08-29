// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The handle through which the app layer reaches BL616-only services.

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use ssh_stamp::config::{SSHStampConfig, UartPins};
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::store;
use ssh_stamp_hal::{FlashError, HalError, OtaActions};

use bl616_wifi::ota::Ota;

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

/// Load the stored configuration, minting a fresh one if the flash is blank.
///
/// # Errors
///
/// Only when a configuration *is* present but fails its version or integrity
/// check. That is deliberately not recoverable here: recreating one would
/// regenerate the SSH host key, breaking client host-key pinning and
/// reopening the unauthenticated first-login window. Erase the config sector
/// to reprovision.
pub async fn load_config(
    default_mac: [u8; 6],
    default_uart_pins: UartPins,
) -> Result<SSHStampConfig, sunset::Error> {
    let mut guard = FLASH.lock().await;
    let (flash, buf) = &mut *guard;
    store::load_or_create(flash, buf, default_mac, default_uart_pins)
}

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

/// The OTA session, which the trait's `&self` writes have to reach.
///
/// One update at a time, and only from the SFTP handler: a second session
/// starting while one is in flight would be two writers on the same slot.
static OTA: Mutex<CriticalSectionRawMutex, Option<Ota>> = Mutex::new(None);

/// Writes an update into the firmware slot that is not running.
///
/// The board's partition table names two slots and says which is live;
/// `bl616-wifi` writes the image into the other one and, at the end, swaps
/// them by publishing a new table. Nothing touches the running image, so an
/// upload that fails or a session that drops leaves the board exactly as it
/// was — the spare slot holds a partial image that nothing will boot.
pub struct Bl616OtaWriter;

/// Anything the flash or the partition table reports, as the HAL spells it.
///
/// The distinction the HAL draws is between "could not write" and "the
/// layout is wrong", and that is the one worth keeping: the first is worth
/// retrying, the second means this board cannot take an update at all.
fn ota_error(e: bl616_wifi::error::Error) -> HalError {
    match e {
        bl616_wifi::error::Error::Partition(_) => HalError::Flash(FlashError::PartitionNotFound),
        bl616_wifi::error::Error::InvalidArgument => HalError::Flash(FlashError::InternalError),
        _ => HalError::Flash(FlashError::Write),
    }
}

impl OtaActions for Bl616OtaWriter {
    /// Tell Boot2 the running image came up.
    ///
    /// Only does anything when the image is on probation — after an update,
    /// on the first boot from the new slot. Every other boot this is a read
    /// of the partition table and nothing more.
    async fn try_validating_current_ota_partition() -> Result<(), HalError> {
        bl616_wifi::ota::confirm_boot().map_err(ota_error)
    }

    /// How large an image the spare slot takes.
    async fn get_ota_partition_size() -> Result<u32, HalError> {
        Ota::begin().map(|ota| ota.capacity()).map_err(ota_error)
    }

    /// Append to the spare slot, starting a session at offset zero.
    ///
    /// Beginning lazily rather than in `get_ota_partition_size` keeps the
    /// session tied to the data actually arriving: a client that asks the
    /// size and then disconnects has claimed nothing.
    async fn write_ota_data(&self, offset: u32, data: &[u8]) -> Result<(), HalError> {
        let mut guard = OTA.lock().await;
        if offset == 0 {
            *guard = Some(Ota::begin().map_err(ota_error)?);
        }
        let ota = guard
            .as_mut()
            .ok_or(HalError::Flash(FlashError::InternalError))?;
        ota.write(offset, data).map_err(ota_error)
    }

    /// Publish a partition table that boots what was just written.
    ///
    /// The single point of no return in an update, and it is one sector
    /// write: before it the board boots the old image, after it the new one.
    async fn finalize_ota_update(&mut self) -> Result<(), HalError> {
        let mut guard = OTA.lock().await;
        let ota = guard
            .take()
            .ok_or(HalError::Flash(FlashError::InternalError))?;
        ota.commit().map_err(ota_error)
    }

    fn reset_device(&self) -> ! {
        bl616_wifi::runtime::reset()
    }
}
