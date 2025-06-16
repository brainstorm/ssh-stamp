use esp_println::{println, dbg};
use esp_storage::FlashStorage;
use embedded_storage::ReadStorage;

use sha2::Digest;

use core::borrow::Borrow;

use embedded_storage::Storage;
use embedded_storage_async::nor_flash::NorFlash;

use sunset::error::Error;
use sunset::sshwire::{self, OwnOrBorrow};

use crate::config::SSHConfig;

// TODO: Adapt those for Espressif targets...
const CONFIG_OFFSET: u32 = 0x150000;
pub const FLASH_SIZE: usize = 2 * 1024 * 1024;

pub(crate) struct Fl {
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

// SSHConfig::CURRENT_VERSION must be bumped if any of this struct #[derive(SSHEncode, SSHDecode)]
struct FlashConfig<'a> {
    version: u8,
    config: OwnOrBorrow<'a, SSHConfig>,
    /// sha256 hash of config
    hash: [u8; 32],
}

impl FlashConfig<'_> {
    const BUF_SIZE: usize = 4 + SSHConfig::BUF_SIZE + 32;
}
const _: () =
    assert!(FlashConfig::BUF_SIZE % 4 == 0, "flash reads must be a multiple of 4");

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
    if let Err(_) = save(flash, &c).await {
        println!("Error writing config");
    }
    Ok(c)
}

pub async fn load(fl: &mut Fl) -> Result<SSHConfig, Error> {
    fl.flash.read(CONFIG_OFFSET, &mut fl.buf).await.map_err(|e| {
        dbg!("flash read error 0x{CONFIG_OFFSET:x} {e:?}");
        Error::msg("flash error")
    })?;

    let s: FlashConfig = sshwire::read_ssh(&fl.buf, None)?;

    if s.version != SSHConfig::CURRENT_VERSION {
        return Err(Error::msg("wrong config version"));
    }

    let calc_hash = config_hash(s.config.borrow())?;
    if calc_hash != s.hash {
        return Err(Error::msg("bad config hash"));
    }

    if let OwnOrBorrow::Own(c) = s.config {
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

    dbg!("flash erase");
    fl.flash
        .erase(CONFIG_OFFSET, CONFIG_OFFSET + ERASE_SIZE as u32)
        .await
        .map_err(|_| Error::msg("flash erase error"))?;

    dbg!("flash write");
    fl.flash
        .write(CONFIG_OFFSET, &buf)
        .await
        .map_err(|_| Error::msg("flash write error"))?;

    println!("flash save done");
    Ok(())
}
