// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};

use ssh_key::sha2::Digest;

use log::{debug, error};

use sunset::error::Error as SunsetError;

use crate::config::{SSHStampConfig, UartPins};

use sunset::sshwire::{self, OwnOrBorrow};
use sunset_sshwire_derive::{SSHDecode, SSHEncode};

// TODO: [Nice to have] Read the right partition and write there instead of hardcoding offset and size.
pub const CONFIG_VERSION_SIZE: usize = 4;
pub const CONFIG_HASH_SIZE: usize = 32;
pub const CONFIG_AREA_SIZE: usize = 4096;
/// Where the configuration area lives in flash.
///
/// A property of the port's flash layout, not of ssh-stamp: the 0x9000
/// default is where an ESP-IDF partition table puts NVS, and it means nothing
/// on a part that does not use one. Set `SSH_STAMP_CONFIG_OFFSET` to move it.
///
/// It stays a `const` because the tests below use it to size arrays.
pub const CONFIG_OFFSET: usize = parse_usize(env!("SSH_STAMP_CONFIG_OFFSET"));

const fn parse_usize(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut value = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        assert!(
            bytes[i].is_ascii_digit(),
            "SSH_STAMP_CONFIG_OFFSET must be a number"
        );
        value = value * 10 + (bytes[i] - b'0') as usize;
        i += 1;
    }
    value
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
}

fn config_hash(config: &SSHStampConfig) -> Result<[u8; 32], SunsetError> {
    let mut h = ssh_key::sha2::Sha256::new();
    sshwire::hash_ser(&mut h, config)?;
    Ok(h.finalize().into())
}

/// Loads a `SSHStampConfig` from flash, or creates a new one if none exists.
///
/// `default_mac` is used only when a new config has to be minted (e.g. first
/// boot); the platform reads this from hardware and passes it in.
///
/// `default_uart_pins` is the target-specific UART pin assignment, used when
/// creating a new config. On subsequent boots the pins are loaded from flash.
///
/// # Errors
/// Returns an error if config creation or flash write fails.
pub fn load_or_create<F>(
    flash: &mut F,
    buf: &mut [u8],
    default_mac: [u8; 6],
    default_uart_pins: UartPins,
) -> Result<SSHStampConfig, SunsetError>
where
    F: NorFlash,
{
    match load_checked(flash, buf) {
        Ok(mut c) => {
            debug!("Good existing config");
            if c.wifi_ap_ssid.as_str() == "ssh-stamp" {
                debug!("Migrating insecure default Access Point SSID, regenerating randomly");
                c.wifi_ap_ssid = SSHStampConfig::generate_wifi_ssid()?;
                if c.wifi_ap_pw.is_empty() {
                    c.wifi_ap_pw = SSHStampConfig::generate_wifi_password()?;
                }
                save(flash, buf, &c)?;
            }
            Ok(c)
        }
        // A config exists but failed the version or integrity check (or the
        // flash read errored). Recreating here would silently wipe the stored
        // pubkeys, regenerate the host key, and reopen the unauthenticated
        // first-login window, so refuse rather than fail open.
        Err(LoadError::Invalid(e)) => {
            error!("Existing config present but invalid; refusing to overwrite it: {e}");
            Err(e)
        }
        // No decodable config at all (blank/erased flash on first boot). This
        // is the only case where minting a fresh config is the right thing.
        Err(LoadError::Absent) => {
            debug!("No existing config found, creating a new one");
            create(flash, buf, default_mac, default_uart_pins)
        }
    }
}

/// Creates a new `SSHStampConfig` and saves it to flash.
///
/// # Errors
/// Returns an error if config creation or flash write fails.
pub fn create<F>(
    flash: &mut F,
    buf: &mut [u8],
    default_mac: [u8; 6],
    default_uart_pins: UartPins,
) -> Result<SSHStampConfig, SunsetError>
where
    F: NorFlash,
{
    let c = SSHStampConfig::new(default_mac, default_uart_pins)?;
    save(flash, buf, &c)?;
    // Don't Debug-print the config: it contains the Ed25519 host private key.
    debug!("Created new config");

    Ok(c)
}

/// Why an existing config could not be loaded from flash.
///
/// Kept as the error half of a `Result` rather than a three-way enum so the
/// large `SSHStampConfig` stays in the `Ok` arm.
enum LoadError {
    /// No decodable config was present (blank/erased flash, e.g. first boot).
    /// This is the only outcome for which minting a fresh config is correct.
    Absent,
    /// A config was structurally present but failed the version or hash check,
    /// or the flash read itself errored. The caller must not overwrite it.
    Invalid(SunsetError),
}

/// Reads and validates the config from flash, distinguishing "no config yet"
/// from "a config is present but invalid" so callers can avoid silently
/// wiping stored keys on the latter.
fn load_checked<F>(flash: &mut F, buf: &mut [u8]) -> Result<SSHStampConfig, LoadError>
where
    F: ReadNorFlash,
{
    // If at some point you target a 64bit arch these can truncate and cause
    // corruption of the bootloader or the ota partition.
    let Ok(offset) = u32::try_from(CONFIG_OFFSET) else {
        return Err(LoadError::Invalid(SunsetError::msg(
            "CONFIG_OFFSET overflow",
        )));
    };

    if flash.read(offset, buf).is_err() {
        error!("flash read error 0x{CONFIG_OFFSET:x}");
        // A transient read error is not proof the config is gone; do not wipe.
        return Err(LoadError::Invalid(SunsetError::msg("flash error")));
    }

    // Undecodable bytes mean no config has been written yet (or the region is
    // erased). This is the only path allowed to fall through to create().
    let Ok((flash_config, _used)) = sshwire::read_ssh::<FlashConfig>(buf, None) else {
        return Err(LoadError::Absent);
    };

    if flash_config.version != SSHStampConfig::CURRENT_VERSION {
        error!("wrong config version on decode: {}", flash_config.version);
        return Err(LoadError::Invalid(SunsetError::msg("wrong config version")));
    }

    // OwnOrBorrow::Own is the only variant that can be decoded from bytes
    let OwnOrBorrow::Own(config) = flash_config.config else {
        return Err(LoadError::Invalid(SunsetError::msg(
            "unexpected borrowed config",
        )));
    };

    let calc_hash = config_hash(&config).map_err(LoadError::Invalid)?;

    if calc_hash != flash_config.hash {
        return Err(LoadError::Invalid(SunsetError::msg("bad config hash")));
    }

    Ok(config)
}

/// Loads `SSHStampConfig` from flash.
///
/// # Errors
/// Returns an error if flash read fails, config is absent, invalid, or the
/// hash mismatches.
pub fn load<F>(flash: &mut F, buf: &mut [u8]) -> Result<SSHStampConfig, SunsetError>
where
    F: ReadNorFlash,
{
    match load_checked(flash, buf) {
        Ok(c) => Ok(c),
        Err(LoadError::Absent) => Err(SunsetError::msg("failed to decode flash config")),
        Err(LoadError::Invalid(e)) => Err(e),
    }
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

    // NB: do not hex_dump `buf` (or the hash) here — the serialized config
    // begins with the Ed25519 host private key and contains the WiFi passwords.
    let l = sshwire::write_ssh(buf, &sc)?;

    debug!("Erasing flash");

    const { assert!(CONFIG_AREA_SIZE > FlashConfig::BUF_SIZE) };

    // Write only the encoded config, rounded up to the flash write
    // granularity, instead of the entire caller buffer. Writing the whole
    // buffer persisted stale trailing RAM to flash and, for a buffer larger
    // than the config area, would write past the erased region into the
    // adjacent partition (NVS/PHY on ESP32).
    let write_len = l
        .checked_next_multiple_of(F::WRITE_SIZE)
        .filter(|n| *n <= buf.len() && *n <= CONFIG_AREA_SIZE)
        .ok_or_else(|| SunsetError::msg("encoded config too large for flash area"))?;

    let offset =
        u32::try_from(CONFIG_OFFSET).map_err(|_| SunsetError::msg("CONFIG_OFFSET overflow"))?;
    let area_size = u32::try_from(CONFIG_AREA_SIZE)
        .map_err(|_| SunsetError::msg("CONFIG_AREA_SIZE overflow"))?;

    flash.erase(offset, offset + area_size).map_err(|_e| {
        error!("flash erase error");
        SunsetError::msg("flash erase error")
    })?;

    flash.write(offset, &buf[..write_len]).map_err(|_e| {
        error!("flash write error");
        SunsetError::msg("flash write error")
    })?;

    debug!("flash save done");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::KEY_SLOTS;
    use core::str::FromStr;
    use embedded_storage::nor_flash::{ErrorType, NorFlashErrorKind, ReadNorFlash};
    use embedded_storage_inmemory::MemFlash;
    use heapless::String;
    use sunset::packets::Ed25519PubKey;
    use sunset::sshwire::Blob;

    type TestFlash = MemFlash<{ CONFIG_OFFSET + CONFIG_AREA_SIZE }, CONFIG_AREA_SIZE, 4>;

    fn round_trip(config: &SSHStampConfig) {
        let mut flash = TestFlash::new(0);
        let mut buf = [0u8; CONFIG_AREA_SIZE];
        save(&mut flash, &mut buf, config).unwrap();
        let loaded = load(&mut flash, &mut buf).unwrap();
        assert_eq!(&loaded, config);
    }

    fn test_config() -> SSHStampConfig {
        SSHStampConfig::new([0x02; 6], UartPins { rx: 10, tx: 11 }).unwrap()
    }

    #[test]
    fn config_with_round_trip() {
        round_trip(&test_config());
    }

    #[test]
    fn config_with_wifi_round_trip() {
        let mut config = test_config();
        config.pubkeys = [Some(Ed25519PubKey {
            key: Blob([0x5a; 32]),
        }); KEY_SLOTS];
        config.wifi_sta_ssid = String::from_str(&"s".repeat(32)).unwrap();
        config.wifi_sta_pw = String::from_str(&"p".repeat(63)).unwrap();

        round_trip(&config);

        let mut buf = [0u8; CONFIG_AREA_SIZE];
        let written = sshwire::write_ssh(
            &mut buf,
            &FlashConfig {
                version: SSHStampConfig::CURRENT_VERSION,
                config: OwnOrBorrow::Borrow(&config),
                hash: config_hash(&config).unwrap(),
            },
        )
        .unwrap();
        assert!(written <= FlashConfig::BUF_SIZE);
        assert!(written <= CONFIG_AREA_SIZE);
    }

    const MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
    /// Matches esp-storage's word-sized writes.
    const WRITE_GRANULARITY: usize = 4;
    /// Enough to cover the config area plus the region that follows it, so a
    /// write running past the erased area is visible to the assertions.
    const FLASH_LEN: usize = CONFIG_OFFSET + 4 * CONFIG_AREA_SIZE;
    /// Stand-in for stale RAM left in the shared flash buffer by an earlier
    /// read; must never reach flash.
    const STALE: u8 = 0xAA;

    fn pins() -> UartPins {
        UartPins { rx: 4, tx: 5 }
    }

    /// NOR-flash stand-in. Erased cells read `0xFF` and a write can only clear
    /// bits (`&=`), as on real NOR, so writing over an unerased cell shows up
    /// as corruption instead of silently succeeding.
    struct MockFlash {
        cells: std::vec::Vec<u8>,
    }

    impl MockFlash {
        fn erased() -> Self {
            Self {
                cells: std::vec![0xFF; FLASH_LEN],
            }
        }

        fn config_area(&self) -> &[u8] {
            &self.cells[CONFIG_OFFSET..CONFIG_OFFSET + CONFIG_AREA_SIZE]
        }

        fn past_config_area(&self) -> &[u8] {
            &self.cells[CONFIG_OFFSET + CONFIG_AREA_SIZE..]
        }

        fn bounds(&self, offset: u32, len: usize) -> Result<(usize, usize), NorFlashErrorKind> {
            let start = offset as usize;
            let end = start
                .checked_add(len)
                .ok_or(NorFlashErrorKind::OutOfBounds)?;
            if end > self.cells.len() {
                return Err(NorFlashErrorKind::OutOfBounds);
            }
            Ok((start, end))
        }
    }

    impl ErrorType for MockFlash {
        type Error = NorFlashErrorKind;
    }

    impl ReadNorFlash for MockFlash {
        const READ_SIZE: usize = 1;

        fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
            let (start, end) = self.bounds(offset, bytes.len())?;
            bytes.copy_from_slice(&self.cells[start..end]);
            Ok(())
        }

        fn capacity(&self) -> usize {
            self.cells.len()
        }
    }

    impl NorFlash for MockFlash {
        const WRITE_SIZE: usize = WRITE_GRANULARITY;
        const ERASE_SIZE: usize = CONFIG_AREA_SIZE;

        fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
            let (start, end) = self.bounds(from, (to - from) as usize)?;
            self.cells[start..end].fill(0xFF);
            Ok(())
        }

        fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
            if !(offset as usize).is_multiple_of(Self::WRITE_SIZE)
                || !bytes.len().is_multiple_of(Self::WRITE_SIZE)
            {
                return Err(NorFlashErrorKind::NotAligned);
            }
            let (start, end) = self.bounds(offset, bytes.len())?;
            for (cell, byte) in self.cells[start..end].iter_mut().zip(bytes) {
                *cell &= byte;
            }
            Ok(())
        }
    }

    /// Length of the encoded `FlashConfig` for `config`, so tests can poke at
    /// the trailing hash without guessing where it lands.
    fn encoded_len(config: &SSHStampConfig) -> usize {
        let sc = FlashConfig {
            version: SSHStampConfig::CURRENT_VERSION,
            config: OwnOrBorrow::Borrow(config),
            hash: config_hash(config).unwrap(),
        };
        let mut probe = [0u8; CONFIG_AREA_SIZE];
        sshwire::write_ssh(&mut probe, &sc).unwrap()
    }

    /// Blank flash must still mint a config: classifying an erased region as
    /// `Invalid` rather than `Absent` would leave a fresh device unable to boot.
    #[test]
    fn blank_flash_mints_a_config() {
        let mut flash = MockFlash::erased();
        let mut buf = [0u8; CONFIG_AREA_SIZE];

        let created = load_or_create(&mut flash, &mut buf, MAC, pins())
            .expect("first boot on erased flash must create a config");
        assert!(created.first_login);

        let mut buf = [0u8; CONFIG_AREA_SIZE];
        let reloaded = load(&mut flash, &mut buf).expect("the config just written must load back");
        assert_eq!(created, reloaded);
    }

    /// A config whose version does not match (e.g. an OTA bumping
    /// `CURRENT_VERSION`) must not be recreated: that would regenerate the host
    /// key and reopen the unauthenticated first-login window.
    #[test]
    fn wrong_version_is_refused_without_overwriting() {
        let mut flash = MockFlash::erased();
        let mut buf = [0u8; CONFIG_AREA_SIZE];
        load_or_create(&mut flash, &mut buf, MAC, pins()).unwrap();

        // The version byte leads the encoded FlashConfig.
        flash.cells[CONFIG_OFFSET] = SSHStampConfig::CURRENT_VERSION.wrapping_add(1);
        let before = flash.cells.clone();

        let mut buf = [0u8; CONFIG_AREA_SIZE];
        assert!(
            load_or_create(&mut flash, &mut buf, MAC, pins()).is_err(),
            "a version mismatch must fail closed, not mint a new config"
        );
        assert_eq!(flash.cells, before, "flash must be left untouched");
    }

    /// Same for a config that fails its integrity check.
    #[test]
    fn bad_hash_is_refused_without_overwriting() {
        let mut flash = MockFlash::erased();
        let mut buf = [0u8; CONFIG_AREA_SIZE];
        let config = load_or_create(&mut flash, &mut buf, MAC, pins()).unwrap();

        // The sha256 occupies the last CONFIG_HASH_SIZE bytes of the record.
        let hash_start = CONFIG_OFFSET + encoded_len(&config) - CONFIG_HASH_SIZE;
        flash.cells[hash_start] ^= 0xFF;
        let before = flash.cells.clone();

        let mut buf = [0u8; CONFIG_AREA_SIZE];
        assert!(
            load_or_create(&mut flash, &mut buf, MAC, pins()).is_err(),
            "a hash mismatch must fail closed, not mint a new config"
        );
        assert_eq!(flash.cells, before, "flash must be left untouched");
    }

    /// `save` must persist only the encoded config, not the rest of the shared
    /// flash buffer, which carries whatever the last read left behind.
    #[test]
    fn save_does_not_persist_stale_buffer_bytes() {
        let mut flash = MockFlash::erased();
        let mut buf = [STALE; CONFIG_AREA_SIZE];
        let config = SSHStampConfig::new(MAC, pins()).unwrap();

        save(&mut flash, &mut buf, &config).unwrap();

        let written = encoded_len(&config).next_multiple_of(WRITE_GRANULARITY);
        assert!(
            flash.config_area()[written..].iter().all(|&b| b == 0xFF),
            "bytes past the encoded config were written to flash"
        );
        // Sanity: the config really is there, so the assertion above is not
        // passing on an empty write.
        let mut buf = [0u8; CONFIG_AREA_SIZE];
        assert_eq!(load(&mut flash, &mut buf).unwrap(), config);
    }

    /// A buffer larger than the config area must not spill past the erased
    /// region into the neighbouring partition.
    #[test]
    fn save_stays_within_the_config_area() {
        let mut flash = MockFlash::erased();
        let mut buf = [STALE; CONFIG_AREA_SIZE * 2];
        let config = SSHStampConfig::new(MAC, pins()).unwrap();

        save(&mut flash, &mut buf, &config).unwrap();

        assert!(
            flash.past_config_area().iter().all(|&b| b == 0xFF),
            "save wrote past CONFIG_AREA_SIZE into the adjacent partition"
        );
    }
}
