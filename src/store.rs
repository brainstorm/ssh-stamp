// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use embedded_storage::ReadStorage;
use embedded_storage::nor_flash::NorFlash;

use pretty_hex::PrettyHex;
use sha2::Digest;

use log::{debug, error};

use sunset::error::Error as SunsetError;

use crate::config::SSHStampConfig;

use sunset::sshwire::{self, OwnOrBorrow};
use sunset_sshwire_derive::{SSHDecode, SSHEncode};

// TODO: [Nice to have] Read the right partition and write there instead of hardcoding offset and size.
pub const CONFIG_VERSION_SIZE: usize = 4;
pub const CONFIG_HASH_SIZE: usize = 32;
pub const CONFIG_AREA_SIZE: usize = 4096;
pub const CONFIG_OFFSET: usize = 0x9000;

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
}

fn config_hash(config: &SSHStampConfig) -> Result<[u8; 32], SunsetError> {
    let mut h = sha2::Sha256::new();
    sshwire::hash_ser(&mut h, config)?;
    Ok(h.finalize().into())
}

/// Loads a `SSHStampConfig` from flash, or creates a new one if none exists.
///
/// `default_mac` is used only when a new config has to be minted (e.g. first
/// boot); the platform reads this from hardware and passes it in.
///
/// # Errors
/// Returns an error if config creation or flash write fails.
pub fn load_or_create<F>(
    flash: &mut F,
    buf: &mut [u8],
    default_mac: [u8; 6],
) -> Result<SSHStampConfig, SunsetError>
where
    F: NorFlash + ReadStorage,
{
    match load(flash, buf) {
        Ok(mut c) => {
            debug!("Good existing config");
            if c.wifi_ssid.as_str() == "ssh-stamp" {
                debug!("Migrating insecure default SSID, regenerating randomly");
                c.wifi_ssid = SSHStampConfig::generate_wifi_ssid()?;
                if c.wifi_pw.is_none() {
                    c.wifi_pw = Some(SSHStampConfig::generate_wifi_password()?);
                }
                save(flash, buf, &c)?;
            }
            return Ok(c);
        }
        Err(e) => debug!("Existing config bad, making new. {e}"),
    }

    create(flash, buf, default_mac)
}

/// Creates a new `SSHStampConfig` and saves it to flash.
///
/// # Errors
/// Returns an error if config creation or flash write fails.
pub fn create<F>(
    flash: &mut F,
    buf: &mut [u8],
    default_mac: [u8; 6],
) -> Result<SSHStampConfig, SunsetError>
where
    F: NorFlash,
{
    let c = SSHStampConfig::new(default_mac)?;
    save(flash, buf, &c)?;
    debug!("Created new config: {:?}", &c);

    Ok(c)
}

/// Loads `SSHStampConfig` from flash.
///
/// # Errors
/// Returns an error if flash read fails, config is invalid, or hash mismatch.
pub fn load<F>(flash: &mut F, buf: &mut [u8]) -> Result<SSHStampConfig, SunsetError>
where
    F: ReadStorage,
{
    // If at some point you target a 64bit arch these can truncate and cause
    // corruption of the bootloader or the ota partition.
    let offset =
        u32::try_from(CONFIG_OFFSET).map_err(|_| SunsetError::msg("CONFIG_OFFSET overflow"))?;

    flash.read(offset, buf).map_err(|_e| {
        error!("flash read error 0x{CONFIG_OFFSET:x}");
        SunsetError::msg("flash error")
    })?;

    let flash_config: FlashConfig = sshwire::read_ssh(buf, None)
        .map_err(|_| SunsetError::msg("failed to decode flash config"))?;

    if flash_config.version != SSHStampConfig::CURRENT_VERSION {
        error!("wrong config version on decode: {}", flash_config.version);
        return Err(SunsetError::msg("wrong config version"));
    }

    // OwnOrBorrow::Own is the only variant that can be decoded from bytes
    let config = match flash_config.config {
        OwnOrBorrow::Own(c) => c,
        OwnOrBorrow::Borrow(_) => return Err(SunsetError::msg("unexpected borrowed config")),
    };

    let calc_hash = config_hash(&config)?;

    if calc_hash != flash_config.hash {
        return Err(SunsetError::msg("bad config hash"));
    }

    Ok(config)
}

/// Saves `SSHStampConfig` to flash.
///
/// # Errors
/// Returns an error if flash write fails or config serialization fails.
pub fn save<F>(flash: &mut F, buf: &mut [u8], config: &SSHStampConfig) -> Result<(), SunsetError>
where
    F: NorFlash,
{
    let sc = FlashConfig {
        version: SSHStampConfig::CURRENT_VERSION,
        config: OwnOrBorrow::Borrow(config),
        hash: config_hash(config)?,
    };

    debug!("Before write_ssh, with hash: {}", &sc.hash.hex_dump());
    let l = sshwire::write_ssh(buf, &sc)?;
    debug!("Saved flash (after write_ssh): {}", &buf[..l].hex_dump());

    debug!(
        "CONFIG_OFFSET + FlashConfig::BUF_SIZE = {}",
        CONFIG_OFFSET + FlashConfig::BUF_SIZE
    );

    debug!("Erasing flash");

    const { assert!(CONFIG_AREA_SIZE > FlashConfig::BUF_SIZE) };

    let offset =
        u32::try_from(CONFIG_OFFSET).map_err(|_| SunsetError::msg("CONFIG_OFFSET overflow"))?;
    let area_size = u32::try_from(CONFIG_AREA_SIZE)
        .map_err(|_| SunsetError::msg("CONFIG_AREA_SIZE overflow"))?;

    flash.erase(offset, offset + area_size).map_err(|_e| {
        error!("flash erase error");
        SunsetError::msg("flash erase error")
    })?;

    flash.write(offset, buf).map_err(|_e| {
        error!("flash write error");
        SunsetError::msg("flash write error")
    })?;

    debug!("flash save done");
    Ok(())
}
