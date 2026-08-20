// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Config storage on the RP2350's XIP flash.
//!
//! [`ssh_stamp::store`] addresses the config at a fixed `CONFIG_OFFSET`
//! (`0x9000`). On the RP2350 that offset sits in the middle of our own
//! program text, so [`ConfigFlash`] translates the store's window onto a
//! dedicated sector at the very top of flash and refuses everything outside
//! it. Nothing else in the image can be reached through this view, which is
//! the point.
//!
//! Only that translating view is local. Pairing it with the store's scratch
//! buffer, and holding the pair in a boot-initialised singleton, is
//! [`ssh_stamp::store::ConfigStore`] / [`ConfigStoreCell`], shared with the
//! other ports.

use embassy_rp::Peri;
use embassy_rp::flash::{Blocking, ERASE_SIZE, Flash};
use embassy_rp::peripherals::FLASH;
use embedded_storage::ReadStorage;
use embedded_storage::nor_flash::{
    ErrorType, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};
use ssh_stamp::store::{CONFIG_AREA_SIZE, CONFIG_OFFSET, ConfigStore, ConfigStoreCell};
use sunset_async::SunsetMutex;

/// Declared flash size. The Pico 2 ships 4 MiB; larger parts just leave the
/// remainder unused. Must match `memory.x`.
pub const FLASH_SIZE: usize = 4 * 1024 * 1024;

/// Same values as `u32`, for address arithmetic without lossy casts. The
/// asserts keep them tied to their `usize` counterparts.
const FLASH_SIZE_ADDR: u32 = 4 * 1024 * 1024;
const ERASE_SIZE_ADDR: u32 = 4096;
const _: () = assert!(FLASH_SIZE_ADDR as usize == FLASH_SIZE);
const _: () = assert!(ERASE_SIZE_ADDR as usize == ERASE_SIZE);

/// The config lives in the last erase sector of the declared flash.
pub const CONFIG_WINDOW: u32 = FLASH_SIZE_ADDR - ERASE_SIZE_ADDR;

const _: () = assert!(
    CONFIG_AREA_SIZE <= ERASE_SIZE,
    "config area must fit in one erase sector"
);

type RpFlash = Flash<'static, FLASH, Blocking, FLASH_SIZE>;

/// Translating view of the config sector. Implements the `embedded-storage`
/// traits `ssh_stamp::store` needs, remapping its fixed `CONFIG_OFFSET` onto
/// [`CONFIG_WINDOW`].
pub struct ConfigFlash {
    flash: RpFlash,
}

impl ConfigFlash {
    #[must_use]
    pub fn new(flash: RpFlash) -> Self {
        Self { flash }
    }
}

/// This port's config store: the translating view plus the store's scratch
/// buffer.
pub type FlashBuffer = ConfigStore<ConfigFlash>;

/// Map a store-relative offset onto the real config sector, rejecting
/// anything that would escape it.
fn translate(offset: u32, len: usize) -> Result<u32, FlashViewError> {
    let base = u32::try_from(CONFIG_OFFSET).map_err(|_| FlashViewError::OutOfBounds)?;
    let area = u32::try_from(CONFIG_AREA_SIZE).map_err(|_| FlashViewError::OutOfBounds)?;
    let len = u32::try_from(len).map_err(|_| FlashViewError::OutOfBounds)?;

    let end = offset.checked_add(len).ok_or(FlashViewError::OutOfBounds)?;
    if offset < base || end > base + area {
        return Err(FlashViewError::OutOfBounds);
    }
    Ok(CONFIG_WINDOW + (offset - base))
}

/// Errors from the translating view: either the access escaped the config
/// window, or the underlying driver failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashViewError {
    OutOfBounds,
    Flash(embassy_rp::flash::Error),
}

impl NorFlashError for FlashViewError {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            FlashViewError::OutOfBounds => NorFlashErrorKind::OutOfBounds,
            FlashViewError::Flash(e) => e.kind(),
        }
    }
}

impl From<embassy_rp::flash::Error> for FlashViewError {
    fn from(e: embassy_rp::flash::Error) -> Self {
        FlashViewError::Flash(e)
    }
}

/// The store's window as seen from outside: `CONFIG_OFFSET + CONFIG_AREA_SIZE`.
/// Reported as the capacity so the store's own bounds checks line up.
const VIRTUAL_CAPACITY: usize = CONFIG_OFFSET + CONFIG_AREA_SIZE;

impl ErrorType for ConfigFlash {
    type Error = FlashViewError;
}

impl ReadNorFlash for ConfigFlash {
    const READ_SIZE: usize = <RpFlash as ReadNorFlash>::READ_SIZE;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let at = translate(offset, bytes.len())?;
        self.flash.read(at, bytes).map_err(FlashViewError::from)
    }

    fn capacity(&self) -> usize {
        VIRTUAL_CAPACITY
    }
}

impl NorFlash for ConfigFlash {
    const WRITE_SIZE: usize = <RpFlash as NorFlash>::WRITE_SIZE;
    const ERASE_SIZE: usize = <RpFlash as NorFlash>::ERASE_SIZE;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        let len = to.checked_sub(from).ok_or(FlashViewError::OutOfBounds)?;
        let at = translate(from, len as usize)?;
        self.flash.erase(at, at + len).map_err(FlashViewError::from)
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let at = translate(offset, bytes.len())?;
        self.flash.write(at, bytes).map_err(FlashViewError::from)
    }
}

// `ssh_stamp::store::load` is generic over `ReadStorage`, so the view has to
// offer that flavour of `read`/`capacity` too.
impl ReadStorage for ConfigFlash {
    type Error = FlashViewError;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        ReadNorFlash::read(self, offset, bytes)
    }

    fn capacity(&self) -> usize {
        VIRTUAL_CAPACITY
    }
}

static FLASH_STORAGE: ConfigStoreCell<ConfigFlash> = ConfigStoreCell::new();

/// Initialise config storage. Call once, early in boot.
pub fn init(flash: Peri<'static, FLASH>) {
    FLASH_STORAGE.init(ConfigFlash::new(Flash::new_blocking(flash)));
}

/// Access the config storage initialised by [`init`].
#[must_use]
pub fn get_flash_n_buffer() -> Option<&'static SunsetMutex<FlashBuffer>> {
    FLASH_STORAGE.get()
}
