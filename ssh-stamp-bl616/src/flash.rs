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
//! This part has a partition table, but not an ESP-IDF one and not a
//! `partitions.csv`: two copies of a binary structure at 0xE000 and 0xF000,
//! written by the flashing tool from the vendor SDK's
//! `bsp/board/<board>/config/partition_cfg_*.toml`, and read by Boot2 to
//! decide what to boot. `bl616-pt` decodes it — that is what the OTA path
//! uses to find the spare firmware slot.
//!
//! The configuration area is the table's `DATA` entry, 20 KB at 0x3F3000 in
//! the 4 MB layout, past the firmware, `mfg` and `media` regions. It reaches
//! this crate as `SSH_STAMP_CONFIG_OFFSET`, which `cargo xtask` sets per
//! board: the ESP-IDF default of 0x9000 lands inside Boot2 here and would
//! brick the board on the first save.

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
