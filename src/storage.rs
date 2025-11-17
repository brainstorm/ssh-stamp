use embedded_storage::ReadStorage;
use esp_bootloader_esp_idf::partitions;
use esp_println::{dbg, println};
use esp_storage::FlashStorage;

use pretty_hex::PrettyHex;
use sha2::Digest;

use core::borrow::Borrow;

use embedded_storage::nor_flash::NorFlash;

use crate::errors::Error as SSHStampError;
use sunset::error::Error as SunsetError;

use sunset::sshwire::{self, OwnOrBorrow};
use sunset_sshwire_derive::*;

use crate::config::SSHStampConfig;

pub const CONFIG_VERSION_SIZE: usize = 4;
pub const CONFIG_HASH_SIZE: usize = 32;
pub const CONFIG_AREA_SIZE: usize = 4096;
pub const CONFIG_OFFSET: usize = 0x9000;

pub struct Fl {
    flash: FlashStorage,
    // Only a single task can write to flash at a time,
    // keeping a buffer here saves duplicated buffer space in each task.
    buf: [u8; FlashConfig::BUF_SIZE],
}

impl<'a> Fl {
    pub fn new(flash: FlashStorage) -> Self {
        Self {
            flash,
            buf: [0u8; FlashConfig::BUF_SIZE],
        }
    }
}

// SSHConfig::CURRENT_VERSION must be bumped if any of this struct
#[derive(SSHEncode, SSHDecode)]
struct FlashConfig<'a> {
    version: u8,
    config: OwnOrBorrow<'a, SSHStampConfig>,
    /// sha256 hash of config
    hash: [u8; 32],
}

impl FlashConfig<'_> {
    const BUF_SIZE: usize = 460; // Must be enough to hold the whole config

    // TODO: Rework Error mapping with esp_storage errors
    /// Finds the NVS partitions and retrieves information about it.
    pub fn find_config_partition() -> Result<(), SSHStampError> {
        let mut flash = FlashStorage::new();
        println!("Flash size = {} bytes", flash.capacity());

        let mut pt_mem = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
        let pt = partitions::read_partition_table(&mut flash, &mut pt_mem).unwrap();
        let nvs = pt
            .find_partition(partitions::PartitionType::Data(
                partitions::DataPartitionSubType::Nvs,
            ))
            .unwrap()
            .unwrap();

        let nvs_partition = nvs.as_embedded_storage(&mut flash);

        println!("NVS partition size = {}", nvs_partition.capacity());
        println!("NVS partition offset = 0x{:x}", nvs.offset());

        Ok(())
    }
}

fn config_hash(config: &SSHStampConfig) -> Result<[u8; 32], SunsetError> {
    let mut h = sha2::Sha256::new();
    sshwire::hash_ser(&mut h, config)?;
    Ok(h.finalize().into())
}

/// Loads a SSHConfig at startup. Good for persisting hostkeys.
pub async fn load_or_create(flash: &mut Fl) -> Result<SSHStampConfig, SunsetError> {
    match load(flash).await {
        Ok(c) => {
            println!("Good existing config");
            return Ok(c);
        }
        Err(e) => println!("Existing config bad, making new. {e}"),
    }

    create(flash).await
}

pub async fn create(flash: &mut Fl) -> Result<SSHStampConfig, SunsetError> {
    let c = SSHStampConfig::new()?;
    save(flash, &c).await?;
    dbg!("Created new config: ", &c);

    Ok(c)
}

pub async fn load(fl: &mut Fl) -> Result<SSHStampConfig, SunsetError> {
    fl.flash
        .read(CONFIG_OFFSET as u32, &mut fl.buf)
        .map_err(|_e| {
            dbg!("flash read error 0x{CONFIG_OFFSET:x} {e:?}");
            SunsetError::msg("flash error")
        })?;

    let flash_config: FlashConfig = sshwire::read_ssh(&fl.buf, None)
        .map_err(|_| SunsetError::msg("failed to decode flash config"))?;

    if flash_config.version != SSHStampConfig::CURRENT_VERSION {
        dbg!("wrong config version on decode: {}", flash_config.version);
        return Err(SunsetError::msg("wrong config version"));
    }

    let calc_hash = config_hash(flash_config.config.borrow()).unwrap();

    if calc_hash != flash_config.hash {
        return Err(SunsetError::msg("bad config hash"));
    }

    if let OwnOrBorrow::Own(c) = flash_config.config {
        Ok(c)
    } else {
        // OK panic - OwnOrBorrow always decodes to Own variant
        panic!()
    }
}

pub async fn save(fl: &mut Fl, config: &SSHStampConfig) -> Result<(), SunsetError> {
    let sc = FlashConfig {
        version: SSHStampConfig::CURRENT_VERSION,
        config: OwnOrBorrow::Borrow(&config),
        hash: config_hash(&config)?,
    };

    FlashConfig::find_config_partition().unwrap();

    //   dbg!("Saving config: ", &config);
    dbg!("Before write_ssh, with hash: ", &sc.hash.hex_dump());
    let l = sshwire::write_ssh(&mut fl.buf, &sc)?;
    let buf = &fl.buf[..l];
    dbg!("Saved flash (after write_ssh): {}", &buf.hex_dump());

    dbg!(CONFIG_OFFSET + FlashConfig::BUF_SIZE);

    dbg!("Erasing flash");

    assert!(CONFIG_AREA_SIZE > FlashConfig::BUF_SIZE);

    fl.flash
        .erase(
            CONFIG_OFFSET as u32,
            (CONFIG_OFFSET + CONFIG_AREA_SIZE) as u32,
        )
        .unwrap();

    fl.flash.write(CONFIG_OFFSET as u32, &fl.buf).unwrap();

    println!("flash save done");
    Ok(())
}
