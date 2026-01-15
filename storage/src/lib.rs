#![no_std]

// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

/// [[flash]] is a packet to provide safe access to the Flash storage used by SSH-Stamp
///
/// It does so by storing the FlashStorage and a buffer for read/write operations in a single structure
/// protected by a SunsetMutex for safe concurrent access in async contexts.
pub mod flash {
    #[allow(unused_imports)]
    use log::{debug, error, info, warn};
    use once_cell::sync::OnceCell;

    use esp_storage::FlashStorage;

    use sunset_async::SunsetMutex;

    const FLASH_BUF_SIZE: usize = esp_storage::FlashStorage::SECTOR_SIZE as usize;

    static FLASH_STORAGE: OnceCell<SunsetMutex<FlashBuffer>> = OnceCell::new();

    /// A structure that holds both the FlashStorage and a buffer for read/write operations
    ///
    /// The buffer is stored here to avoid allocating multiple buffers in different parts of the code.
    /// It has a fixed size defined by FLASH_BUF_SIZE.
    #[derive(Debug)]
    pub struct FlashBuffer {
        pub flash: FlashStorage,
        pub buf: [u8; FLASH_BUF_SIZE],
    }

    impl FlashBuffer {
        pub fn new(flash: FlashStorage) -> Self {
            Self {
                flash,
                buf: [0u8; FLASH_BUF_SIZE],
            }
        }

        /// For cases where it is necessary to use both flash and buffer mutably at the same time
        pub fn split_ref_mut(&mut self) -> (&mut FlashStorage, &mut [u8]) {
            (&mut self.flash, &mut self.buf)
        }
    }

    /// Shall be called at startup to avoid lazy initialization during runtime
    ///
    /// Calls to [`with_flash`] or [`get_flash`] will initialize the flash storage if not already done.
    ///
    /// Multiple calls to init() are safe and will have no effect after the first one.
    pub fn init() {
        if FLASH_STORAGE.get().is_some() {
            return;
        }
        let fl = FlashBuffer::new(FlashStorage::new());
        FLASH_STORAGE
            .set(SunsetMutex::new(fl))
            .expect("Flash storage already initialized");
    }
    /// Lazy access the flash storage mutex and run the provided closure with a mutable reference to it.
    ///
    /// To avoid the lazy initialization, call [`init()`] at startup.
    pub async fn with_flash_n_buffer<F, R>(f: F) -> R
    where
        F: FnOnce(&mut FlashBuffer) -> R,
    {
        init();
        let flash_ref = FLASH_STORAGE.get().expect("Flash storage not initialized");
        let mut guard = flash_ref.lock().await;
        f(&mut guard)
    }

    /// Lazy static accessor for the flash storage mutex. It will initialize it if not already done.
    ///
    /// It is expected that the user will drop the lock on the mutex after use...
    ///
    /// To avoid the lazy initialization, call [`init()`] at startup.
    pub fn get_flash_n_buffer() -> &'static SunsetMutex<FlashBuffer> {
        init();
        FLASH_STORAGE.get().expect("Flash storage not initialized")
    }
}
