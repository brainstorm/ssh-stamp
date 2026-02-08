#![no_std]
// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

/// Module defining the ESP32-specific storage traits implementations for OTA updates
#[cfg(any(
    feature = "esp32",
    feature = "esp32s2",
    feature = "esp32s3",
    feature = "esp32c3",
    feature = "esp32c6"
))]
pub mod esp;

// TODO: When the time comes, generalise the flash so it can be used with all supported targets
/// [[flash]] is a packet to provide safe access to the Flash storage used by SSH-Stamp
///
/// It does so by storing the FlashStorage and a buffer for read/write operations in a single structure
/// protected by a SunsetMutex for safe concurrent access in async contexts.
pub mod flash {
    use esp_hal::peripherals::FLASH;
    use esp_storage::FlashStorage;
    const FLASH_BUF_SIZE: usize = esp_storage::FlashStorage::SECTOR_SIZE as usize;
    static FLASH_STORAGE: OnceCell<SunsetMutex<FlashBuffer>> = OnceCell::new();

    #[allow(unused_imports)]
    use log::{debug, error, info, warn};
    use once_cell::sync::OnceCell;
    use sunset_async::SunsetMutex;

    /// A structure that holds both the FlashStorage and a buffer for read/write operations
    ///
    /// The buffer is stored here to avoid allocating multiple buffers in different parts of the code.
    /// It has a fixed size defined by FLASH_BUF_SIZE.
    #[derive(Debug)]
    pub struct FlashBuffer<'d> {
        pub flash: FlashStorage<'d>,
        pub buf: [u8; FLASH_BUF_SIZE],
    }

    impl<'d> FlashBuffer<'d> {
        pub fn new(flash: FlashStorage<'static>) -> Self {
            Self {
                flash,
                buf: [0u8; FLASH_BUF_SIZE],
            }
        }

        /// For cases where it is necessary to use both flash and buffer mutably at the same time
        pub fn split_ref_mut(&mut self) -> (&mut FlashStorage<'d>, &mut [u8]) {
            (&mut self.flash, &mut self.buf)
        }
    }

    /// Shall be called at startup to avoid lazy initialization during runtime
    ///
    /// Calls to [`with_flash`] or [`get_flash`] will initialize the flash storage if not already done.
    ///
    /// Multiple calls to init() are safe and will have no effect after the first one.
    pub fn init(flash: FLASH<'static>) {
        let fl = FlashBuffer::new(FlashStorage::new(flash));

        let Ok(()) = FLASH_STORAGE.set(SunsetMutex::new(fl)) else {
            warn!("Flash storage already initialized");
            return;
        };
    }

    /// Static accessor for the flash storage mutex. Warning: It will fail if not initialized.
    ///
    /// call [`init()`] at startup.
    ///
    /// It is expected that the user will drop the lock on the mutex after use...
    pub fn get_flash_n_buffer() -> Option<&'static SunsetMutex<FlashBuffer<'static>>> {
        FLASH_STORAGE.get()
    }
}
