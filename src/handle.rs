// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use heapless::String;
use log::{debug, warn};
use sunset_async::SunsetMutex;

use crate::config::SSHStampConfig;

pub mod env_parser {
    use super::String;

    /// Limit the maximum length accepted for an SSH key, Ed25519 lines
    /// should be less than this.
    const PUBKEY_MAX_LEN: usize = 256;

    /// Sanitizes environment variable input by checking for valid ASCII graphic characters.
    ///
    /// Returns `true` if the input contains at least one character and all characters
    /// are ASCII graphic characters (printable characters excluding space).
    #[must_use]
    pub fn env_sanitize(s: &str) -> bool {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_graphic())
    }

    /// Validates a public key environment value.
    ///
    /// This accepts printable ASCII, including spaces, as the format
    /// for a key expects `<type> <base64> [comment]`. This would be
    /// rejected by `env_sanitize` which is stricter, so it is separated
    /// out here for pubkey environment variables only.
    #[must_use]
    pub fn parse_pubkey(value: &str) -> Option<&str> {
        let trimmed = value.trim();

        if trimmed.is_empty() || trimmed.len() > PUBKEY_MAX_LEN {
            return None;
        }
        if !trimmed.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
            return None;
        }

        Some(trimmed)
    }

    #[must_use]
    pub fn parse_wifi_ssid(value: &str) -> Option<String<32>> {
        if !env_sanitize(value) {
            return None;
        }
        let mut s = String::new();
        s.push_str(value).ok()?;
        Some(s)
    }

    #[must_use]
    pub fn parse_wifi_psk(value: &str) -> Option<String<63>> {
        if value.len() < 8 || value.len() > 63 {
            return None;
        }
        if !env_sanitize(value) {
            return None;
        }
        let mut s = String::new();
        s.push_str(value).ok()?;
        Some(s)
    }

    #[must_use]
    pub fn parse_mac_address(value: &str) -> Option<[u8; 6]> {
        if !env_sanitize(value) {
            return None;
        }
        if value.len() != 17 {
            return None;
        }
        let parts: heapless::Vec<u8, 6> = value
            .split(':')
            .filter_map(|p| u8::from_str_radix(p, 16).ok())
            .collect();
        if parts.len() != 6 {
            return None;
        }
        Some([parts[0], parts[1], parts[2], parts[3], parts[4], parts[5]])
    }
}

/// Handles `SSH_STAMP_PUBKEY` environment variable requests.
///
/// # Errors
/// Returns an error if SSH protocol operations fail or if the pubkey cannot be added.
pub async fn pubkey_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    config_changed: &mut bool,
    auth_checked: &mut bool,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;

    if !config_guard.first_login {
        warn!("SSH_STAMP_PUBKEY env received but not first-login; rejecting");
        a.fail()?;
    } else {
        match env_parser::parse_pubkey(a.value()?) {
            None => {
                warn!("SSH_STAMP_PUBKEY contains invalid characters");
                a.fail()?;
            }
            Some(trimmed) => {
                if config_guard.add_pubkey(trimmed).is_ok() {
                    debug!("Added new pubkey from ENV");
                    a.succeed()?;
                    config_guard.first_login = false;
                    *config_changed = true;
                    *auth_checked = true;
                } else {
                    warn!("Failed to add new pubkey from ENV");
                    a.fail()?;
                }
            }
        }
    }
    Ok(())
}

/// Handles `SSH_STAMP_WIFI_SSID` environment variable requests.
///
/// # Errors
/// Returns an error if SSH protocol operations fail or if the SSID is invalid.
pub async fn wifi_ssid_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    config_changed: &mut bool,
    needs_reset: &mut bool,
    auth_checked: bool,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if auth_checked || config_guard.first_login {
        if let Some(s) = env_parser::parse_wifi_ssid(a.value()?) {
            config_guard.wifi_ssid = s;
            debug!("Set wifi SSID from ENV");
            a.succeed()?;
            *config_changed = true;
            *needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_SSID invalid and/or too long");
            a.fail()?;
        }
    } else {
        warn!("SSH_STAMP_WIFI_SSID env received but not authenticated; rejecting");
        a.fail()?;
    }
    Ok(())
}

/// Handles `SSH_STAMP_WIFI_PSK` environment variable requests.
///
/// # Errors
/// Returns an error if SSH protocol operations fail or if the PSK is invalid.
pub async fn wifi_psk_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    config_changed: &mut bool,
    needs_reset: &mut bool,
    auth_checked: bool,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if auth_checked || config_guard.first_login {
        if let Some(s) = env_parser::parse_wifi_psk(a.value()?) {
            config_guard.wifi_pw = Some(s);
            debug!("Set WIFI PSK from ENV");
            a.succeed()?;
            *config_changed = true;
            *needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_PSK invalid and/or not within 8-63 characters");
            a.fail()?;
        }
    } else {
        warn!("SSH_STAMP_WIFI_PSK env received but not authenticated; rejecting");
        a.fail()?;
    }
    Ok(())
}

/// Handles `SSH_STAMP_WIFI_MAC_ADDRESS` environment variable requests.
///
/// # Errors
/// Returns an error if SSH protocol operations fail or if the MAC address is invalid.
pub async fn wifi_mac_address_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    config_changed: &mut bool,
    needs_reset: &mut bool,
    auth_checked: bool,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if auth_checked || config_guard.first_login {
        if let Some(mac) = env_parser::parse_mac_address(a.value()?) {
            config_guard.mac = mac;
            debug!("Set MAC address from ENV: {mac:02X?}");
            a.succeed()?;
            *config_changed = true;
            *needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_MAC_ADDRESS must be XX:XX:XX:XX:XX:XX format");
            a.fail()?;
        }
    } else {
        warn!("SSH_STAMP_WIFI_MAC_ADDRESS env received but not authenticated; rejecting");
        a.fail()?;
    }
    Ok(())
}

/// Handles `SSH_STAMP_WIFI_MAC_RANDOM` environment variable requests.
///
/// # Errors
/// Returns an error if SSH protocol operations fail or if authentication is missing.
pub async fn wifi_mac_random_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    config_changed: &mut bool,
    needs_reset: &mut bool,
    auth_checked: bool,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if auth_checked || config_guard.first_login {
        config_guard.mac = [0xFF; 6];
        debug!("Set MAC address to random mode");
        a.succeed()?;
        *config_changed = true;
        *needs_reset = true;
    } else {
        warn!("SSH_STAMP_WIFI_MAC_RANDOM env received but not authenticated; rejecting");
        a.fail()?;
    }
    Ok(())
}
