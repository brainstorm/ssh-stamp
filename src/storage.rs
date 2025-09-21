use esp_println::{println, dbg};
use esp_storage::FlashStorage;
use embedded_storage::ReadStorage;

use pretty_hex::PrettyHex;
use sha2::Digest;

use core::borrow::Borrow;

use embedded_storage::nor_flash::NorFlash;

use sunset::error::Error;
use sunset::sshwire::{self, OwnOrBorrow};
use sunset_sshwire_derive::*;

use crate::config::SSHConfig;

pub const CONFIG_VERSION_SIZE: usize = 4;
pub const CONFIG_HASH_SIZE: usize = 32;
pub const CONFIG_AREA_SIZE: usize = 8192;
pub const CONFIG_OFFSET: usize = 0x9000;

pub struct Fl {
    flash: FlashStorage,
    // Only a single task can write to flash at a time,
    // keeping a buffer here saves duplicated buffer space in each task.
    buf: [u8; FlashConfig::BUF_SIZE],
}

impl<'a> Fl {
    pub fn new(flash: FlashStorage) -> Self {
        Self { flash, buf: [0u8; FlashConfig::BUF_SIZE] }
    }
}

// SSHConfig::CURRENT_VERSION must be bumped if any of this struct
#[derive(SSHEncode, SSHDecode)]
struct FlashConfig<'a> {
    version: u8,
    config: OwnOrBorrow<'a, SSHConfig>,
    /// sha256 hash of config
    hash: [u8; 32],
}

impl FlashConfig<'_> {
    const BUF_SIZE: usize = CONFIG_VERSION_SIZE + CONFIG_AREA_SIZE + CONFIG_HASH_SIZE;
}

// TODO: Does not match esp_storage assumptions about SECTOR_SIZE alignment? 
//
// const _: () =
//     assert!(FlashConfig::BUF_SIZE % 4 == 0, "flash reads must be a multiple of 4");

fn config_hash(config: &SSHConfig) -> Result<[u8; 32], Error> {
    let mut h = sha2::Sha256::new();
    sshwire::hash_ser(&mut h, config)?;
    Ok(h.finalize().into())
}

/// Loads a SSHConfig at startup. Good for persisting hostkeys.
pub async fn load_or_create(flash: &mut Fl) -> Result<SSHConfig, Error> {
    match load(flash).await {
        Ok(c) => {
            println!("Good existing config");
            return Ok(c);
        }
        Err(e) => println!("Existing config bad, making new. {e}"),
    }

    create(flash).await
}

pub async fn create(flash: &mut Fl) -> Result<SSHConfig, Error> {
    let c = SSHConfig::new()?;
    save(flash, &c).await?;
 
    Ok(c)
}

pub async fn load(fl: &mut Fl) -> Result<SSHConfig, Error> {
    fl.flash.read(CONFIG_OFFSET as u32, &mut fl.buf).map_err(|_e| {
        dbg!("flash read error 0x{CONFIG_OFFSET:x} {e:?}");
        Error::msg("flash error")
    })?;

    let flash_config: FlashConfig = sshwire::read_ssh(&fl.buf, None)
        .map_err(|_| Error::msg("failed to decode flash config"))?;

    if flash_config.version != SSHConfig::CURRENT_VERSION {
        return Err(Error::msg("wrong config version"));
    }

    let calc_hash = config_hash(flash_config.config.borrow())?;
    if calc_hash != flash_config.hash {
        return Err(Error::msg("bad config hash"));
    }

    if let OwnOrBorrow::Own(c) = flash_config.config {
        Ok(c)
    } else {
        // OK panic - OwnOrBorrow always decodes to Own variant
        panic!()
    }
}

pub async fn save(fl: &mut Fl, config: &SSHConfig) -> Result<(), Error> {
    let sc = FlashConfig {
        version: SSHConfig::CURRENT_VERSION,
        config: OwnOrBorrow::Borrow(&config),
        hash: config_hash(&config)?,
    };
    let l = sshwire::write_ssh(&mut fl.buf, &sc)?;
    let buf = &fl.buf[..l];

    dbg!(&buf.hex_dump());

    fl.flash
        //.erase(CONFIG_OFFSET as u32, (CONFIG_OFFSET + CONFIG_AREA_SIZE + 32) as u32)
        .erase(CONFIG_OFFSET as u32, (CONFIG_OFFSET + CONFIG_AREA_SIZE) as u32).unwrap();
        //.map_err(|_| Error::msg("flash erase error"))?;

    // TODO: Add padding to &buf to satisfy underlying nor_flash alignment requirements (4096-aligned SECTOR_SIZE)

    fl.flash
        .write(CONFIG_OFFSET as u32, &buf).unwrap();
        //.map_err(|_| Error::msg("flash write error"))?;

    println!("flash save done");
    Ok(())
}
