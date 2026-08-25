// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! This provisions device config on from the host, which is used to
//! deterministically specify the AP config.

use crate::device::SshAuth;
use crate::host::AccessPoint;
use anyhow::{Context, Result, anyhow, bail};
use embedded_storage_inmemory::MemFlash;
use getrandom::fill;
use ssh_key::LineEnding;
use ssh_key::private::{Ed25519Keypair, KeypairData, PrivateKey};
use ssh_key::public::{Ed25519PublicKey, KeyData, PublicKey};
use ssh_stamp::config::{SSHStampConfig, UartPins};
use ssh_stamp::store::{self, CONFIG_AREA_SIZE, CONFIG_OFFSET};
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use sunset::SignKey;
use sunset::packets::Ed25519PubKey;
use sunset::sshwire::Blob;
use tempfile::{TempDir, tempdir};

/// This is an in-memory version of the flash area that the firmware would write.
type ImageFlash = MemFlash<{ CONFIG_OFFSET + CONFIG_AREA_SIZE }, CONFIG_AREA_SIZE, 4>;

/// The per-run identity that a provisioned device boots with.
pub struct Provision {
    /// The access point config.
    access_point: AccessPoint,
    /// The `app_config` image.
    image: PathBuf,
    /// The `known_hosts` file.
    known_hosts: PathBuf,
    /// The client private key.
    identity: PathBuf,
    /// Holds files until dropped.
    _dir: TempDir,
}

impl Provision {
    /// Creates credentials for the device deterministically by provisioning the
    /// SSID and password.
    pub fn generate(host: &str, mac: [u8; 6], uart_pins: UartPins) -> Result<Self> {
        let mut config =
            SSHStampConfig::new(mac, uart_pins).map_err(|e| anyhow!("could not provision: {e}"))?;

        // Create a dedicated keypair for the ssh connection.
        let client = Self::client_keypair()?;
        config.pubkeys[0] = Some(Ed25519PubKey {
            key: Blob(client.public.0),
        });
        // The client key is already enrolled, so no first-login window.
        config.first_login = false;

        let access_point = AccessPoint {
            ssid: config.wifi_ap_ssid.as_str().to_string(),
            psk: config.wifi_ap_pw.as_str().to_string(),
        };

        let dir = tempdir().context("could not create provisioning directory")?;
        let image = dir.path().join("app_config.bin");
        fs::write(&image, Self::config_image(&config)?).context("could not write config image")?;

        // This allows the ssh connection to trust a known host, i.e. the board itself.
        let known_hosts = dir.path().join("known_hosts");
        fs::write(&known_hosts, Self::known_hosts_line(host, &config)?)
            .context("could not write known_hosts")?;

        // Write the private key so that the follow up ssh connections can use it.
        let identity = dir.path().join("id_ed25519");
        Self::write_private_key(&identity, client)?;

        Ok(Provision {
            access_point,
            image,
            known_hosts,
            identity,
            _dir: dir,
        })
    }

    /// Get the access point from the provisioning.
    pub fn access_point(&self) -> &AccessPoint {
        &self.access_point
    }

    /// The `app_config` partition image to flash.
    pub fn image(&self) -> &Path {
        &self.image
    }

    /// The pinned settings for the SSH sessions of this run.
    pub fn ssh_auth(&self) -> SshAuth {
        SshAuth {
            known_hosts: self.known_hosts.clone(),
            identity: self.identity.clone(),
        }
    }

    /// Generate a new Ed25519 keypair.
    pub fn client_keypair() -> Result<Ed25519Keypair> {
        let mut seed = [0u8; 32];
        fill(&mut seed).map_err(|e| anyhow!("could not generate the keypair: {e}"))?;
        Ok(Ed25519Keypair::from_seed(&seed))
    }

    /// Serializes the `config` in order to write to the board, exactly like the board
    /// would format it.
    pub fn config_image(config: &SSHStampConfig) -> Result<Vec<u8>> {
        let mut flash = Box::new(ImageFlash::new(0xFF));

        let mut buf = [0u8; CONFIG_AREA_SIZE];
        store::save(flash.as_mut(), &mut buf, config)
            .map_err(|e| anyhow!("could not serialize config: {e}"))?;

        Ok(flash.mem[CONFIG_OFFSET..].to_vec())
    }

    /// Format the `known_hosts` line that allows the host to trust the firmware board.
    pub fn known_hosts_line(host: &str, config: &SSHStampConfig) -> Result<String> {
        let SignKey::Ed25519(key) = &config.hostkey else {
            bail!("the host key is not Ed25519");
        };
        let public = PublicKey::new(
            KeyData::Ed25519(Ed25519PublicKey(key.verifying_key().to_bytes())),
            "",
        );

        Ok(format!("{host} {}\n", public.to_openssh()?))
    }

    /// Writes the client private key in the format that OpenSSH will use to connect
    /// to the device.
    pub fn write_private_key(path: &Path, keypair: Ed25519Keypair) -> Result<()> {
        let key = PrivateKey::new(KeypairData::Ed25519(keypair), "")?;

        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        // ssh refuses a key that other users can read, so create it as 0600.
        #[cfg(unix)]
        options.mode(0o600);

        options
            .open(path)
            .context("could not create the client key")?
            .write_all(key.to_openssh(LineEnding::LF)?.as_bytes())
            .context("could not write the client key")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssh_stamp::store::load;

    #[test]
    fn create_image_like_firmware_would() {
        let mac = [0x98, 0xa3, 0x16, 0x96, 0x8c, 0x08];
        let provision =
            Provision::generate("192.168.4.1", mac, UartPins { rx: 10, tx: 11 }).unwrap();
        let image = fs::read(provision.image()).unwrap();
        assert_eq!(image.len(), CONFIG_AREA_SIZE);

        let mut flash = Box::new(ImageFlash::new(0xFF));
        flash.mem[CONFIG_OFFSET..].copy_from_slice(&image);
        let mut buf = [0u8; CONFIG_AREA_SIZE];
        let config = load(flash.as_mut(), &mut buf).unwrap();

        assert!(!config.first_login);
        assert!(config.pubkeys[0].is_some());
        assert_eq!(config.wifi_ap_ssid.as_str(), provision.access_point.ssid);
        assert_eq!(config.wifi_ap_pw.as_str(), provision.access_point.psk);
        assert_eq!(config.uart_pins, UartPins { rx: 10, tx: 11 });
        assert_eq!(config.mac, mac);
    }
}
