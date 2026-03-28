// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use heapless::String;
use log::{debug, info, warn};

use crate::config::SSHStampConfig;
use crate::espressif::buffered_uart::UART_SIGNAL;
use crate::serial::serial_bridge;
use crate::store;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use esp_hal::system::software_reset;
use hal_espressif::flash;

use core::fmt::Debug;
use core::option::Option::None;
use core::result::Result;

use sunset::{ChanHandle, ServEvent};
use sunset_async::{ChanInOut, SSHServer, SunsetMutex};

pub mod env_parser {
    use super::String;

    /// Sanitizes environment variable input by checking for valid ASCII graphic characters.
    ///
    /// Returns `true` if the input contains at least one character and all characters
    /// are ASCII graphic characters (printable characters excluding space).
    #[must_use]
    pub fn env_sanitize(s: &str) -> bool {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_graphic())
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

#[derive(Debug)]
pub enum SessionType {
    Bridge(ChanHandle),
    #[cfg(feature = "sftp-ota")]
    Sftp(ChanHandle),
}

pub struct EventContext<'a> {
    pub session: &'a mut Option<ChanHandle>,
    pub auth_checked: &'a mut bool,
    pub config_changed: &'a mut bool,
    pub needs_reset: &'a mut bool,
}

pub fn session_subsystem(
    ev: ServEvent<'_, '_>,
    ctx: &mut EventContext<'_>,
    _chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
) -> Result<(), sunset::Error> {
    if let ServEvent::SessionSubsystem(a) = ev {
        debug!("ServEvent::SessionSubsystem");

        if !*ctx.auth_checked {
            warn!("Unauthenticated SessionSubsystem rejected");
            a.fail()?;
        } else if a.command()?.to_lowercase().as_str() == "sftp" {
            if let Some(ch) = ctx.session.take() {
                debug_assert!(ch.num() == a.channel());
                #[cfg(feature = "sftp-ota")]
                {
                    a.succeed()?;
                    debug!("We got SFTP subsystem");
                    match _chan_pipe.try_send(SessionType::Sftp(ch)) {
                        Ok(_) => *ctx.auth_checked = false,
                        Err(e) => log::error!("Could not send the channel: {:?}", e),
                    };
                }
                #[cfg(not(feature = "sftp-ota"))]
                {
                    warn!("SFTP subsystem requested but not supported in this build");
                    a.fail()?;
                }
            } else {
                a.fail()?;
            }
        } else {
            a.fail()?;
        }
    }
    Ok(())
}

pub async fn session_shell(
    ev: ServEvent<'_, '_>,
    ctx: &mut EventContext<'_>,
    config: &SunsetMutex<SSHStampConfig>,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
) -> Result<(), sunset::Error> {
    if let ServEvent::SessionShell(a) = ev {
        debug!("ServEvent::SessionShell");

        if !*ctx.auth_checked {
            warn!("Unauthenticated SessionShell rejected");
            a.fail()?;
        } else if let Some(ch) = ctx.session.take() {
            if *ctx.config_changed {
                *ctx.config_changed = false;
                let config_guard = config.lock().await;
                let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
                    panic!("Could not acquire flash storage lock");
                };
                let mut flash_storage = flash_storage_guard.lock().await;
                let _result = store::save(&mut flash_storage, &config_guard).await;
                drop(config_guard);
                if *ctx.needs_reset {
                    info!("Configuration saved. Rebooting to apply WiFi changes...");
                    software_reset();
                }
            }
            debug_assert!(ch.num() == a.channel());
            a.succeed()?;
            debug!("We got shell");
            UART_SIGNAL.signal(1);
            debug!("Connection loop: UART_SIGNAL sent");
            match chan_pipe.try_send(SessionType::Bridge(ch)) {
                Ok(_) => *ctx.auth_checked = false,
                Err(e) => log::error!("Could not send the channel: {:?}", e),
            };
        } else {
            a.fail()?;
        }
    }
    Ok(())
}

pub async fn first_auth(
    ev: ServEvent<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
) -> Result<(), sunset::Error> {
    if let ServEvent::FirstAuth(mut a) = ev {
        debug!("ServEvent::FirstAuth");
        let config_guard = config.lock().await;

        a.enable_password_auth(false)?;

        a.enable_pubkey_auth(true)?;
        if config_guard.first_login {
            a.allow()?;
        } else {
            debug!(
                "FirstAuth received but not first-login, rejecting"
            );
            a.reject()?;
        }
    }
    Ok(())
}

pub async fn hostkeys(
    ev: ServEvent<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
) -> Result<(), sunset::Error> {
    if let ServEvent::Hostkeys(h) = ev {
        debug!("ServEvent::Hostkeys");
        let config_guard = config.lock().await;
        h.hostkeys(&[&config_guard.hostkey])?;
    }
    Ok(())
}

pub fn password_auth(ev: ServEvent<'_, '_>) -> Result<(), sunset::Error> {
    if let ServEvent::PasswordAuth(a) = ev {
        warn!("Password auth is not supported, use public key auth instead.");
        a.reject()?;
    }
    Ok(())
}

pub async fn pubkey_auth(
    ev: ServEvent<'_, '_>,
    ctx: &mut EventContext<'_>,
    config: &SunsetMutex<SSHStampConfig>,
) -> Result<(), sunset::Error> {
    if let ServEvent::PubkeyAuth(a) = ev {
        debug!("ServEvent::PubkeyAuth");
        let config_guard = config.lock().await;
        let client_pubkey = a.pubkey()?;

        match client_pubkey {
            sunset::packets::PubKey::Ed25519(presented) => {
                let matched = config_guard
                    .pubkeys
                    .iter()
                    .any(|slot| slot.as_ref().is_some_and(|stored| *stored == presented));

                if matched {
                    a.allow()?;
                    *ctx.auth_checked = true;
                } else {
                    debug!("No matching pubkey slot found");
                    a.reject()?;
                }
            }
            _ => {
                a.reject()?;
            }
        }
    }
    Ok(())
}

pub fn open_session(
    ev: ServEvent<'_, '_>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    if let ServEvent::OpenSession(a) = ev {
        debug!("ServEvent::OpenSession");
        match ctx.session {
            Some(_) => {
                todo!("Can't have two sessions");
            }
            None => {
                *ctx.session = Some(a.accept()?);
            }
        }
    }
    Ok(())
}

pub async fn session_env(
    ev: ServEvent<'_, '_>,
    ctx: &mut EventContext<'_>,
    config: &SunsetMutex<SSHStampConfig>,
) -> Result<(), sunset::Error> {
    if let ServEvent::SessionEnv(a) = ev {
        debug!("Got ENV request");
        debug!("ENV name: {}", a.name()?);
        debug!("ENV value: {}", a.value()?);

        match a.name()? {
            "LANG" => {
                a.succeed()?;
            }
            "SSH_STAMP_PUBKEY" => {
                pubkey_env(a, config, ctx).await?;
            }
            "SSH_STAMP_WIFI_SSID" => {
                wifi_ssid_env(a, config, ctx).await?;
            }
            "SSH_STAMP_WIFI_PSK" => {
                wifi_psk_env(a, config, ctx).await?;
            }
            "SSH_STAMP_WIFI_MAC_ADDRESS" => {
                wifi_mac_address_env(a, config, ctx).await?;
            }
            "SSH_STAMP_WIFI_MAC_RANDOM" => {
                wifi_mac_random_env(a, config, ctx).await?;
            }
            _ => {
                debug!("Ignoring unknown environment variable: {}", a.name()?);
                a.succeed()?;
            }
        }
    }
    Ok(())
}

/// Handles `SSH_STAMP_PUBKEY` environment variable requests.
///
/// # Errors
/// Returns an error if SSH protocol operations fail or if the pubkey cannot be added.
pub async fn pubkey_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;

    if !config_guard.first_login {
        warn!("SSH_STAMP_PUBKEY env received but not first-login; rejecting");
        a.fail()?;
    } else if !env_parser::env_sanitize(a.value()?) {
        warn!("SSH_STAMP_PUBKEY contains invalid characters");
        a.fail()?;
    } else if config_guard.add_pubkey(a.value()?).is_ok() {
        debug!("Added new pubkey from ENV");
        a.succeed()?;
        config_guard.first_login = false;
        *ctx.config_changed = true;
        *ctx.auth_checked = true;
    } else {
        warn!("Failed to add new pubkey from ENV");
        a.fail()?;
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
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if *ctx.auth_checked || config_guard.first_login {
        if let Some(s) = env_parser::parse_wifi_ssid(a.value()?) {
            config_guard.wifi_ssid = s;
            debug!("Set wifi SSID from ENV");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
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
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if *ctx.auth_checked || config_guard.first_login {
        if let Some(s) = env_parser::parse_wifi_psk(a.value()?) {
            config_guard.wifi_pw = Some(s);
            debug!("Set WIFI PSK from ENV");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
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
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if *ctx.auth_checked || config_guard.first_login {
        if let Some(mac) = env_parser::parse_mac_address(a.value()?) {
            config_guard.mac = mac;
            debug!("Set MAC address from ENV: {mac:02X?}");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
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
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if *ctx.auth_checked || config_guard.first_login {
        config_guard.mac = [0xFF; 6];
        debug!("Set MAC address to random mode");
        a.succeed()?;
        *ctx.config_changed = true;
        *ctx.needs_reset = true;
    } else {
        warn!("SSH_STAMP_WIFI_MAC_RANDOM env received but not authenticated; rejecting");
        a.fail()?;
    }
    Ok(())
}

pub async fn session_pty(
    ev: ServEvent<'_, '_>,
    ctx: &mut EventContext<'_>,
    config: &SunsetMutex<SSHStampConfig>,
) -> Result<(), sunset::Error> {
    if let ServEvent::SessionPty(a) = ev {
        let first_login = { config.lock().await.first_login };

        if *ctx.auth_checked || first_login {
            debug!("ServEvent::SessionPty: Session granted");
            a.succeed()?;
        } else {
            debug!("ServEvent::SessionPty: No auth not session");
            a.fail()?;
        }
    }
    Ok(())
}

pub fn session_exec(ev: ServEvent<'_, '_>) -> Result<(), sunset::Error> {
    if let ServEvent::SessionExec(a) = ev {
        a.fail()?;
    }
    Ok(())
}

pub fn defunct() -> Result<(), sunset::Error> {
    debug!("Expected caller to handle event");
    sunset::error::BadUsage.fail()
}

pub async fn ssh_client<'a, 'b>(
    uart_buff: &'a crate::espressif::buffered_uart::BufferedUart,
    ssh_server: &'b SSHServer<'a>,
    chan_pipe: &'b Channel<NoopRawMutex, SessionType, 1>,
) -> Result<(), sunset::Error> {
    debug!("Preparing bridge");
    let session_type = chan_pipe.receive().await;
    debug!("Checking bridge session type");
    match session_type {
        SessionType::Bridge(ch) => {
            info!("Handling bridge session");
            let stdio: ChanInOut<'_> = ssh_server.stdio(ch).await?;
            let (stdin, stdout) = stdio.split();
            info!("Starting bridge");
            serial_bridge(stdin, stdout, uart_buff).await?
        }
        #[cfg(feature = "sftp-ota")]
        SessionType::Sftp(ch) => {
            debug!("Handling SFTP session");
            let stdio = ssh_server.stdio(ch).await?;
            let ota_writer = hal_espressif::EspOtaWriter::new();
            ota::run_ota_server::<hal_espressif::EspOtaWriter>(stdio, ota_writer).await?
        }
    };
    Ok(())
}

pub async fn bridge_disable() {
    debug!("Bridge disabled: WIP");
}