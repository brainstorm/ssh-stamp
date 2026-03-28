//! Flash storage and OTA implementation for ESP32 family
//!
//! Provides access to flash storage for configuration persistence and firmware updates.

use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_bootloader_esp_idf;
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;
use hal::{FlashError, FlashHal, HalError, OtaActions};
use log::{debug, error};
use once_cell::sync::OnceCell;
use sunset_async::SunsetMutex;

const FLASH_BUF_SIZE: usize = FlashStorage::SECTOR_SIZE as usize;

/// Flash storage singleton
static FLASH_STORAGE: OnceCell<SunsetMutex<FlashBuffer<'static>>> = OnceCell::new();

/// Flash buffer holding both storage and read/write buffer
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

    /// Get mutable references to both flash and buffer
    pub fn split_ref_mut(&mut self) -> (&mut FlashStorage<'d>, &mut [u8]) {
        (&mut self.flash, &mut self.buf)
    }
}

/// Initialize flash storage
pub fn init(flash: FLASH<'static>) {
    let fl = FlashBuffer::new(FlashStorage::new(flash));

    let Ok(()) = FLASH_STORAGE.set(SunsetMutex::new(fl)) else {
        log::warn!("Flash storage already initialized");
        return;
    };
}

/// Get flash storage and buffer
pub fn get_flash_n_buffer() -> Option<&'static SunsetMutex<FlashBuffer<'static>>> {
    FLASH_STORAGE.get()
}

/// ESP Flash implementation
pub struct EspFlash;

impl FlashHal for EspFlash {
    async fn read(&self, offset: u32, buf: &mut [u8]) -> Result<(), HalError> {
        let Some(fb) = get_flash_n_buffer() else {
            return Err(HalError::Flash(FlashError::Read));
        };
        let mut fb = fb.lock().await;

        fb.flash
            .read(offset, buf)
            .map_err(|_| HalError::Flash(FlashError::Read))
    }

    async fn write(&self, offset: u32, buf: &[u8]) -> Result<(), HalError> {
        let Some(fb) = get_flash_n_buffer() else {
            return Err(HalError::Flash(FlashError::Write));
        };
        let mut fb = fb.lock().await;

        NorFlash::write(&mut fb.flash, offset, buf).map_err(|_| HalError::Flash(FlashError::Write))
    }

    async fn erase(&self, offset: u32, len: u32) -> Result<(), HalError> {
        let Some(fb) = get_flash_n_buffer() else {
            return Err(HalError::Flash(FlashError::Erase));
        };
        let mut fb = fb.lock().await;

        NorFlash::erase(&mut fb.flash, offset, len).map_err(|_| HalError::Flash(FlashError::Erase))
    }

    fn size(&self) -> u32 {
        // ESP32 flash size varies by chip, return a reasonable default
        // Actual size can be read from efuse in production
        4 * 1024 * 1024 // 4MB default
    }
}

/// OTA writer for ESP32
#[derive(Debug, Copy, Clone)]
pub struct EspOtaWriter {}

impl EspOtaWriter {
    pub fn new() -> Self {
        EspOtaWriter {}
    }

    async fn next_ota_size() -> Result<u32, HalError> {
        let Some(fb) = get_flash_n_buffer() else {
            error!("Flash storage not initialized");
            return Err(HalError::Flash(FlashError::InternalError));
        };
        let mut fb = fb.lock().await;

        let (storage, _buffer) = fb.split_ref_mut();
        let mut buff_ota = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];

        let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_ota)
            .map_err(|_| HalError::Flash(FlashError::InternalError))?;
        let (target_partition, _) = ota
            .next_partition()
            .map_err(|_| HalError::Flash(FlashError::InternalError))?;

        Ok(target_partition.partition_size() as u32)
    }

    async fn write_to_target(offset: u32, data: &[u8]) -> Result<(), HalError> {
        let Some(fb) = get_flash_n_buffer() else {
            error!("Flash storage not initialized");
            return Err(HalError::Flash(FlashError::InternalError));
        };
        let mut fb = fb.lock().await;

        let (storage, _buffer) = fb.split_ref_mut();
        let mut buff_ota = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];

        let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_ota)
            .map_err(|_| HalError::Flash(FlashError::InternalError))?;
        let (mut target_partition, part_type) = ota
            .next_partition()
            .map_err(|_| HalError::Flash(FlashError::InternalError))?;

        debug!("Flashing image to {:?}", part_type);
        debug!(
            "Writing data to target_partition at offset {}, with len {}",
            offset,
            data.len()
        );

        NorFlash::write(&mut target_partition, offset, data)
            .map_err(|_| HalError::Flash(FlashError::Write))?;

        Ok(())
    }

    async fn activate_next_ota_slot() -> Result<(), HalError> {
        let Some(fb) = get_flash_n_buffer() else {
            error!("Flash storage not initialized");
            return Err(HalError::Flash(FlashError::InternalError));
        };
        let mut fb = fb.lock().await;

        let (storage, _buffer) = fb.split_ref_mut();
        let mut buff_ota = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];

        let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_ota)
            .map_err(|_| HalError::Flash(FlashError::InternalError))?;

        ota.activate_next_partition()
            .map_err(|_| HalError::Flash(FlashError::Write))?;
        ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::New)
            .map_err(|_| HalError::Flash(FlashError::Write))?;

        Ok(())
    }
}

impl Default for EspOtaWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl OtaActions for EspOtaWriter {
    async fn try_validating_current_ota_partition() -> Result<(), HalError> {
        let Some(fb) = get_flash_n_buffer() else {
            error!("Flash storage not initialized");
            return Err(HalError::Flash(FlashError::InternalError));
        };
        let mut fb = fb.lock().await;

        let (storage, _buffer) = fb.split_ref_mut();
        let mut buff_ota = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];

        let mut ota = esp_bootloader_esp_idf::ota_updater::OtaUpdater::new(storage, &mut buff_ota)
            .map_err(|_| HalError::Flash(FlashError::InternalError))?;
        let _current = ota
            .selected_partition()
            .map_err(|_| HalError::Flash(FlashError::InternalError))?;

        debug!("current image state {:?}", ota.current_ota_state());

        let state_result = ota.current_ota_state();
        if let Ok(state) = state_result {
            if state == esp_bootloader_esp_idf::ota::OtaImageState::New
                || state == esp_bootloader_esp_idf::ota::OtaImageState::PendingVerify
            {
                ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::Valid)
                    .map_err(|_| HalError::Flash(FlashError::Write))?;
                debug!("Changed state to VALID");
            }
        }

        Ok(())
    }

    async fn get_ota_partition_size() -> Result<u32, HalError> {
        Self::next_ota_size().await
    }

    async fn write_ota_data(&self, offset: u32, data: &[u8]) -> Result<(), HalError> {
        Self::write_to_target(offset, data).await
    }

    async fn finalize_ota_update(&mut self) -> Result<(), HalError> {
        Self::activate_next_ota_slot().await
    }

    fn reset_device(&self) -> ! {
        esp_hal::system::software_reset()
    }
}
