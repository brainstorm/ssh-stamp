// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The configuration area, as an `embedded-storage` device.
//!
//! `ssh-stamp`'s `store` reads and writes through `ReadNorFlash`/`NorFlash`,
//! so this adapts the BL616's raw flash to that. Offsets are absolute into
//! the flash chip, which is what `store` passes and what the vendor API
//! expects, so nothing is rebased here.
//!
//! # Layout
//!
//! `SSH_STAMP_CONFIG_OFFSET` decides where the configuration lives. On this
//! part there is no ESP-IDF partition table, so the default 0x9000 is
//! meaningless and the board build sets it explicitly to a sector that the
//! firmware image does not occupy.

use bl616_wifi::flash;
use embedded_storage::nor_flash::{
    ErrorType, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};

/// A flash operation the vendor driver refused.
#[derive(Debug)]
pub struct Bl616FlashError;

impl NorFlashError for Bl616FlashError {
    fn kind(&self) -> NorFlashErrorKind {
        NorFlashErrorKind::Other
    }
}

/// The whole SPI flash, addressed by absolute offset.
pub struct Bl616Flash;

impl ErrorType for Bl616Flash {
    type Error = Bl616FlashError;
}

impl ReadNorFlash for Bl616Flash {
    /// Reads are byte-addressable on this part.
    const READ_SIZE: usize = 1;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        flash::read(offset, bytes).map_err(|_| Bl616FlashError)
    }

    fn capacity(&self) -> usize {
        // 4 MB, matching the linker script's `rom` region. Reported rather
        // than probed: `store` only uses it to bounds-check, and a wrong
        // answer here would reject a valid offset rather than corrupt one.
        4 * 1024 * 1024
    }
}

impl NorFlash for Bl616Flash {
    /// Writes go through the vendor driver, which handles page boundaries.
    const WRITE_SIZE: usize = 1;
    /// Erase is per sector, and erasing anything erases all of it.
    const ERASE_SIZE: usize = flash::SECTOR_SIZE as usize;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        flash::erase(from, to.saturating_sub(from)).map_err(|_| Bl616FlashError)
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        flash::write(offset, bytes).map_err(|_| Bl616FlashError)
    }
}
