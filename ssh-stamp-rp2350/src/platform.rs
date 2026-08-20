// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! RP2350 implementation of [`PlatformServices`].

use ssh_stamp::config::SSHStampConfig;
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::store;
use ssh_stamp_hal::{FlashError, HalError, OtaActions};

use crate::flash;
use crate::uart::UART_SIGNAL;

/// Zero-sized handle through which the app layer reaches RP2350 services.
pub struct Rp2350Platform;

impl Rp2350Platform {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for Rp2350Platform {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformServices for Rp2350Platform {
    type OtaWriter = Rp2350OtaWriter;

    async fn save_config(&self, config: &SSHStampConfig) -> Result<(), HalError> {
        let Some(guard) = flash::get_flash_n_buffer() else {
            return Err(HalError::Flash(FlashError::InternalError));
        };
        let mut view = guard.lock().await;
        let (flash, buf) = view.split_ref_mut();
        store::save(flash, buf, config).map_err(|_| HalError::Flash(FlashError::Write))
    }

    fn reset(&self) -> ! {
        cortex_m::peripheral::SCB::sys_reset()
    }

    fn ota_writer(&self) -> Self::OtaWriter {
        Rp2350OtaWriter
    }

    fn activate_uart(&self) {
        UART_SIGNAL.signal(1);
    }
}

/// OTA is not implemented on this port.
///
/// Staging an image would be easy enough (the flash driver is right there),
/// but *activating* it is not: the RP2350 boot ROM picks its image from a
/// partition table, and this port ships neither an A/B layout nor a
/// second-stage bootloader. Rather than accept an upload that could never
/// boot — and risk bricking a board mid-write — every operation refuses
/// up front. Reflash over USB (BOOTSEL/UF2) or SWD instead. The `sftp-ota`
/// feature is deliberately not wired up for this crate.
///
/// TODO(#125): wire up OTA over SFTP, as the ESP32 port has. The blocker is
/// the image layout, not the transport: it needs an RP2350 partition table
/// with two slots inside the declared 4 MiB (see `flash.rs`, which currently
/// hands the config the top sector and nothing else), a `finalize` that
/// rewrites the boot selection, and a rollback path for an image that comes
/// up dead. Only once that exists can these methods do anything but refuse.
pub struct Rp2350OtaWriter;

impl OtaActions for Rp2350OtaWriter {
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
        cortex_m::peripheral::SCB::sys_reset()
    }
}
