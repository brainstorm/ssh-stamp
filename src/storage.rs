use esp_println::{println, dbg};
use esp_storage::FlashStorage;
use embedded_storage::ReadStorage;

use sha2::Digest;

use core::borrow::Borrow;

use embedded_storage::nor_flash::NorFlash;

use sunset::error::Error;
use sunset::sshwire;
use sunset::sshwire::OwnOrBorrow;
use sunset_sshwire_derive::*;

use crate::config::SSHConfig;

// TODO: Adapt those for Espressif targets...
pub const CONFIG_AREA_SIZE: usize = 460;
const CONFIG_OFFSET: u32 = 0x110000;

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
    const BUF_SIZE: usize = 4 + CONFIG_AREA_SIZE + 32;
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
    fl.flash.read(CONFIG_OFFSET, &mut fl.buf).map_err(|_e| {
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
    // TODO: Adapt 4096, ERASE_SIZE in rp, what's in Espressif?
        .erase(CONFIG_OFFSET, CONFIG_OFFSET + 4096 as u32)
        .map_err(|_| Error::msg("flash erase error"))?;

    dbg!("flash write");
    fl.flash
        .write(CONFIG_OFFSET, &buf)
        .map_err(|_| Error::msg("flash write error"))?;

    println!("flash save done");
    Ok(())
}

/// Alternative function demonstrating how to properly handle the SSHSource trait bound issue
/// This shows the correct way to call FlashConfig::dec if needed
fn parse_flash_config_from_buffer(buf: &[u8]) -> Result<FlashConfig<'_>, Error> {
    // CORRECT: Use sshwire::read_ssh which handles the SSHSource trait implementation
    let config: FlashConfig = sshwire::read_ssh(buf, None)?;
    
    // If someone was trying to do this (WRONG):
    // let config: FlashConfig = FlashConfig::dec(&mut buf)?; // ERROR: [u8]: SSHSource<'_> not satisfied
    
    // The fix is to use sshwire::read_ssh instead of calling dec directly
    // sshwire::read_ssh internally creates a DecodeBytes struct that implements SSHSource
    
    Ok(config)
}

/// Example function demonstrating the proper way to use FlashConfig::dec
/// The key insight is that SSHSource is implemented for DecodeBytes, not raw slices
#[cfg(test)]
fn demonstrate_sshsource_usage() -> Result<(), Error> {
    let _buf = [0u8; 100];
    
    // WRONG: This causes the error "the trait bound `[u8]: SSHSource<'_>` is not satisfied"  
    // let s: FlashConfig = FlashConfig::dec(&mut buf)?; 
    
    // CORRECT APPROACH 1: Use sshwire::read_ssh (recommended)
    // let s: FlashConfig = sshwire::read_ssh(&buf, None)?;
    
    // CORRECT APPROACH 2: If you must use FlashConfig::dec directly, 
    // you need to create a proper SSHSource implementation
    // This is internal to sunset crate, so usually you should use read_ssh
    
    Ok(())
}
