// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! SSH event handlers: authentication, channels, environment variables.
//!
//! Every incoming SSH event is dispatched here by the connection loop in
//! [`serve`](crate::serve). The main entry point is [`session_env`], which
//! routes environment variable requests to handlers like [`pubkey_env`] and
//! [`wifi_ap_ssid_env`].
//!
//! First-boot provisioning also flows through here: when `first_login` is true,
//! the device accepts any SSH connection (empty password) and allows the
//! client to set `SSH_STAMP_PUBKEY`. Subsequent connections require that key.
//!
//! Handlers report back to the client by queueing a [`notice!`](crate::notice)
//! on [`EventContext::notices`]; see [`crate::notices`] for why those go to
//! SSH stderr rather than the session's stdout.

use heapless::String;
use log::{debug, info, warn};

use crate::config::SSHStampConfig;
use crate::notice;
use crate::notices::{self, NoticeDrain, Notices, PreAuth, band_label};
use crate::platform::PlatformServices;
use crate::serial::{BufferedSerial, serial_bridge};

#[cfg(feature = "can")]
use crate::can::can_bridge;

use core::cell::RefCell;
#[cfg(feature = "can")]
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;

use core::result::Result;

use embedded_io_async::Write as _;
use sunset::packets::PubKey;
use sunset::{ChanFail, ChanHandle, DisconnectReason, ServEvent};
#[cfg(feature = "can")]
use sunset_async::ChanInOut;
use sunset_async::{SSHServer, SunsetMutex};

pub mod env_parser {
    use super::String;
    use core::str::FromStr;

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

    /// Parses and validates a `WiFi` SSID from an environment variable value.
    ///
    /// Returns `None` if the value contains non-ASCII-graphic characters.
    #[must_use]
    pub fn parse_wifi_ap_ssid(value: &str) -> Option<String<32>> {
        if !env_sanitize(value) {
            return None;
        }
        let mut s = String::new();
        s.push_str(value).ok()?;
        Some(s)
    }

    /// Parses and validates a `WiFi` SSID from an environment variable value.
    ///
    /// Returns `None` if the value contains non-ASCII-graphic characters.
    #[must_use]
    pub fn parse_wifi_station_ssid(value: &str) -> Option<String<32>> {
        if !value.is_empty() && !env_sanitize(value) {
            return None;
        }
        let mut s = String::new();
        s.push_str(value).ok()?;
        Some(s)
    }

    /// Parses and validates a `WiFi` PSK from an environment variable value.
    ///
    /// Returns `None` if the value is not between 8 and 63 characters
    /// or contains non-ASCII-graphic characters.
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

    /// Parses a MAC address from an environment variable value in `XX:XX:XX:XX:XX:XX` format.
    ///
    /// Returns `None` if the value is not exactly 17 characters, contains
    /// non-hex-colon characters, or does not produce exactly 6 octets.
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

    /// Parses a `WiFi` band mode from an environment variable value.
    ///
    /// Accepts: `"2.4g"`, `"5g"`, `"auto"` (case-insensitive).
    /// Returns the band as a `u8`: 0 = 2.4GHz, 1 = 5GHz, 2 = Auto.
    #[must_use]
    pub fn parse_wifi_band(value: &str) -> Option<u8> {
        ssh_stamp_hal::BandMode::from_str(value)
            .ok()
            .map(|band| band as u8)
    }
}

#[derive(Debug)]
pub enum SessionType {
    Bridge(ChanHandle),
    #[cfg(feature = "sftp-ota")]
    Sftp(ChanHandle),
}

/// Per-connection queue of messages bound for the client's stderr.
///
/// Shared between the connection loop (which produces most notices during
/// session setup) and the bridge (which drains them once a channel exists).
pub type NoticeQueue = BlockingMutex<NoopRawMutex, RefCell<Notices>>;

pub struct EventContext<'a> {
    pub session: &'a mut Option<ChanHandle>,
    /// Messages to hand the client once there is a channel to write on.
    pub notices: &'a NoticeQueue,
    /// Set by [`first_auth`] for the connection loop to send once the
    /// session mutex is free; see [`PreAuth`].
    pub pre_auth: &'a mut Option<PreAuth>,
    pub auth_checked: &'a mut bool,
    pub config_changed: &'a mut bool,
    pub needs_reset: &'a mut bool,
    /// Hands accepted `can` subsystem channels to the CAN bridge, which
    /// runs concurrently with the shell (UART) session.
    #[cfg(feature = "can")]
    pub can_queue: &'a Channel<NoopRawMutex, ChanHandle, 1>,
    /// Set once a CAN session is dispatched on this connection. SFTP (OTA)
    /// needs the connection's full bandwidth, so it is refused afterwards.
    #[cfg(all(feature = "sftp-ota", feature = "can"))]
    pub can_dispatched: &'a mut bool,
}

/// Handles SSH session subsystem requests (e.g., SFTP, CAN).
///
/// # Errors
///
/// Returns an error if SSH protocol operations fail.
pub fn session_subsystem(
    ev: ServEvent<'_, '_>,
    ctx: &mut EventContext<'_>,
    #[cfg(feature = "sftp-ota")] chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
) -> Result<(), sunset::Error> {
    if let ServEvent::SessionSubsystem(a) = ev {
        debug!("ServEvent::SessionSubsystem");

        if !*ctx.auth_checked {
            warn!("Unauthenticated SessionSubsystem rejected");
            a.fail()?;
        } else if a.command()?.to_lowercase().as_str() == "sftp" {
            #[cfg(feature = "sftp-ota")]
            {
                // SFTP (OTA) is exclusive: it needs the connection's full
                // bandwidth, so refuse it once a CAN session is active.
                #[cfg(feature = "can")]
                let can_active = *ctx.can_dispatched;
                #[cfg(not(feature = "can"))]
                let can_active = false;
                if can_active {
                    warn!("SFTP subsystem refused: a CAN session is active on this connection");
                    notice!(
                        ctx.notices,
                        "sftp refused: a CAN session already owns this connection"
                    );
                    a.fail()?;
                } else if let Some(ch) = ctx.session.take() {
                    debug_assert_eq!(ch.num(), a.channel());
                    a.succeed()?;
                    debug!("We got SFTP subsystem");
                    match chan_pipe.try_send(SessionType::Sftp(ch)) {
                        Ok(()) => *ctx.auth_checked = false,
                        Err(e) => log::error!("Could not send the channel: {e:?}"),
                    }
                } else {
                    a.fail()?;
                }
            }
            #[cfg(not(feature = "sftp-ota"))]
            {
                warn!("SFTP subsystem requested but not supported in this build");
                notice!(
                    ctx.notices,
                    "sftp refused: this firmware was built without the sftp-ota feature"
                );
                a.fail()?;
            }
        } else if a.command()?.to_lowercase().as_str() == "can" {
            #[cfg(feature = "can")]
            if let Some(ch) = ctx.session.take() {
                debug_assert_eq!(ch.num(), a.channel());
                a.succeed()?;
                debug!("We got CAN subsystem");
                // auth_checked is deliberately left untouched so the same
                // (already authenticated) connection can still request a
                // shell session and bridge UART concurrently with CAN.
                if let Err(e) = ctx.can_queue.try_send(ch) {
                    log::error!("Could not send the CAN channel: {e:?}");
                }
                #[cfg(feature = "sftp-ota")]
                {
                    *ctx.can_dispatched = true;
                }
            } else {
                a.fail()?;
            }
            #[cfg(not(feature = "can"))]
            {
                warn!("CAN subsystem requested but not supported in this build");
                notice!(
                    ctx.notices,
                    "can refused: this firmware was built without the can feature"
                );
                a.fail()?;
            }
        } else {
            a.fail()?;
        }
    }
    Ok(())
}

/// Handles SSH session shell requests.
///
/// # Errors
///
/// Returns an error if SSH protocol operations fail.
pub async fn session_shell<P: PlatformServices>(
    ev: ServEvent<'_, '_>,
    ctx: &mut EventContext<'_>,
    config: &SunsetMutex<SSHStampConfig>,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
    platform: &P,
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
                platform
                    .save_config(&config_guard)
                    .await
                    .map_err(|_| sunset::error::BadUsage.build())?;
                drop(config_guard);
                if *ctx.needs_reset {
                    info!("Configuration saved. Rebooting to apply WiFi changes...");
                    // Nothing to notify on: the reset happens before this
                    // channel opens, so the client just sees the connection
                    // drop. Documented in docs/USING.md.
                    platform.reset();
                }
                notice!(ctx.notices, "config: saved to flash");
            }
            debug_assert_eq!(ch.num(), a.channel());
            a.succeed()?;
            debug!("We got shell");
            platform.activate_uart();
            debug!("Connection loop: UART activated");
            match chan_pipe.try_send(SessionType::Bridge(ch)) {
                Ok(()) => *ctx.auth_checked = false,
                Err(e) => log::error!("Could not send the channel: {e:?}"),
            }
        } else {
            a.fail()?;
        }
    }
    Ok(())
}

/// Handles the first authentication request.
///
/// # Errors
///
/// Returns an error if SSH protocol operations fail.
pub async fn first_auth(
    ev: ServEvent<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    if let ServEvent::FirstAuth(mut a) = ev {
        debug!("ServEvent::FirstAuth");
        let config_guard = config.lock().await;

        // Fires once per connection, before any key is offered, so this is
        // the one point where the device can describe itself to a client
        // that may never authenticate. Sent by the connection loop, which
        // is not holding the session mutex.
        *ctx.pre_auth = Some(notices::preauth_for(&config_guard));

        a.enable_password_auth(false)?;

        a.enable_pubkey_auth(true)?;
        if config_guard.first_login {
            a.allow()?;
        } else {
            debug!("FirstAuth received but not first-login, rejecting");
            a.reject()?;
        }
    }
    Ok(())
}

/// Provides host keys to the SSH client.
///
/// # Errors
///
/// Returns an error if SSH protocol operations fail.
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

/// Rejects password authentication requests.
///
/// # Errors
///
/// Returns an error if SSH protocol operations fail.
pub fn password_auth(ev: ServEvent<'_, '_>) -> Result<(), sunset::Error> {
    if let ServEvent::PasswordAuth(a) = ev {
        warn!("Password auth is not supported, use public key auth instead.");
        a.reject()?;
    }
    Ok(())
}

/// Handles SSH public key authentication.
///
/// # Errors
///
/// Returns an error if SSH protocol operations fail.
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
            PubKey::Ed25519(presented) => {
                let matched = config_guard
                    .pubkeys
                    .iter()
                    .any(|slot| slot.as_ref().is_some_and(|stored| *stored == presented));

                if matched {
                    *ctx.auth_checked = true;
                    a.allow()?;
                } else {
                    debug!("No matching pubkey slot found");
                    a.reject()?;
                }
            }
            PubKey::Unknown(_) => {
                a.reject()?;
            }
        }
    }
    Ok(())
}

/// Handles SSH session open requests, rejecting duplicates.
///
/// # Errors
///
/// Returns an error if SSH protocol operations fail.
pub fn open_session(
    ev: ServEvent<'_, '_>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    if let ServEvent::OpenSession(a) = ev {
        debug!("ServEvent::OpenSession");
        match ctx.session {
            Some(_) => {
                warn!("Rejecting duplicate session channel");
                a.reject(ChanFail::SSH_OPEN_ADMINISTRATIVELY_PROHIBITED)?;
            }
            None => {
                *ctx.session = Some(a.accept()?);
            }
        }
    }
    Ok(())
}

/// Handles `SSH_STAMP_NOTICES` environment variable requests.
///
/// Notices already ride on SSH stderr, so a client that wants them out of
/// the way can simply redirect. This exists for clients that cannot split
/// the two streams — `ssh -t`, and anything merging them before ssh-stamp
/// sees the difference.
///
/// Needs no authentication: it only decides whether the device talks, and
/// silence is always safe to grant.
///
/// # Errors
/// Returns an error if SSH protocol operations fail.
pub fn notices_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    match a.value()? {
        "off" | "0" | "false" | "no" => {
            ctx.notices.lock(|n| n.borrow_mut().disable());
            debug!("Client turned notices off");
            a.succeed()
        }
        "on" | "1" | "true" | "yes" => {
            // Notices are on by default; accepting this makes the variable
            // safe to set unconditionally in a wrapper script.
            a.succeed()
        }
        other => {
            warn!("SSH_STAMP_NOTICES must be on or off, got {other:?}");
            a.fail()
        }
    }
}

/// Handles SSH environment variable requests.
///
/// # Errors
///
/// Returns an error if SSH protocol operations fail.
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
            "SSH_STAMP_WIFI_AP_SSID" => {
                wifi_ap_ssid_env(a, config, ctx).await?;
            }
            "SSH_STAMP_WIFI_AP_PSK" => {
                wifi_ap_psk_env(a, config, ctx).await?;
            }
            "SSH_STAMP_WIFI_BAND" => {
                wifi_band_env(a, config, ctx).await?;
            }
            "SSH_STAMP_WIFI_STA_SSID" => {
                wifi_sta_ssid_env(a, config, ctx).await?;
            }
            "SSH_STAMP_WIFI_STA_PW" => {
                wifi_sta_psk_env(a, config, ctx).await?;
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
///
/// Returns an error if SSH protocol operations fail or if the pubkey cannot be added.
pub async fn pubkey_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;

    if config_guard.first_login {
        match env_parser::parse_pubkey(a.value()?) {
            None => {
                warn!("SSH_STAMP_PUBKEY contains invalid characters");
                notice!(
                    ctx.notices,
                    "SSH_STAMP_PUBKEY rejected: not a valid ed25519 public key"
                );
                a.fail()?;
            }
            Some(trimmed) => {
                if config_guard.add_pubkey(trimmed).is_ok() {
                    debug!("Added new pubkey from ENV");
                    notice!(
                        ctx.notices,
                        "config: authorised key added; first-login provisioning is now closed"
                    );
                    a.succeed()?;
                    if config_guard.first_login {
                        config_guard.first_login = false;
                        *ctx.config_changed = true;
                        *ctx.auth_checked = true;
                    }
                } else {
                    warn!("Failed to add new pubkey from ENV");
                    notice!(ctx.notices, "SSH_STAMP_PUBKEY rejected: no free key slot");
                    a.fail()?;
                }
            }
        }
    } else {
        warn!("SSH_STAMP_PUBKEY env received but not first-login; rejecting");
        a.fail()?;
    }

    Ok(())
}

/// Handles `SSH_STAMP_WIFI_AP_SSID` environment variable requests.
///
/// # Errors
/// Returns an error if SSH protocol operations fail or if the SSID is invalid.
pub async fn wifi_ap_ssid_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if *ctx.auth_checked || config_guard.first_login {
        if let Some(s) = env_parser::parse_wifi_ap_ssid(a.value()?) {
            notice!(
                ctx.notices,
                "config: wifi ap ssid {:?} -> {:?}",
                config_guard.wifi_ap_ssid.as_str(),
                s.as_str()
            );
            config_guard.wifi_ap_ssid = s;
            debug!("Set wifi Access Point SSID from ENV");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_AP_SSID invalid and/or too long");
            notice!(
                ctx.notices,
                "SSH_STAMP_WIFI_AP_SSID rejected: empty or over 32 bytes"
            );
            a.fail()?;
        }
    } else {
        warn!("SSH_STAMP_WIFI_AP_SSID env received but not authenticated; rejecting");
        a.fail()?;
    }
    Ok(())
}

/// Handles `SSH_STAMP_WIFI_AP_PSK` environment variable requests.
///
/// # Errors
/// Returns an error if SSH protocol operations fail or if the PSK is invalid.
pub async fn wifi_ap_psk_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if *ctx.auth_checked || config_guard.first_login {
        if let Some(s) = env_parser::parse_wifi_psk(a.value()?) {
            notice!(
                ctx.notices,
                "config: wifi ap psk updated ({} chars)",
                s.len()
            );
            config_guard.wifi_ap_pw = s;
            debug!("Set WIFI AP PSK from ENV");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_AP_PSK invalid and/or not within 8-63 characters");
            notice!(
                ctx.notices,
                "SSH_STAMP_WIFI_AP_PSK rejected: must be 8-63 characters"
            );
            a.fail()?;
        }
    } else {
        warn!("SSH_STAMP_WIFI_AP_PSK env received but not authenticated; rejecting");
        a.fail()?;
    }
    Ok(())
}

/// Handles `SSH_STAMP_WIFI_BAND` environment variable requests.
///
/// Accepts `2.4g`, `5g`, or `auto` (case-insensitive). Only the ESP32-C5
/// supports 5GHz; other chips will ignore the setting at runtime.
/// Triggers a config save + reset on success.
///
/// # Errors
/// Returns an error if SSH protocol operations fail.
pub async fn wifi_band_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if *ctx.auth_checked || config_guard.first_login {
        if let Some(band) = env_parser::parse_wifi_band(a.value()?) {
            notice!(
                ctx.notices,
                "config: wifi ap band {} -> {}",
                band_label(config_guard.wifi_ap_band),
                band_label(band)
            );
            config_guard.wifi_ap_band = band;
            debug!("Set WIFI AP band from ENV: {band}");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_BAND must be 2.4g, 5g, or auto");
            notice!(
                ctx.notices,
                "SSH_STAMP_WIFI_BAND rejected: must be 2.4g, 5g or auto"
            );
            a.fail()?;
        }
    } else {
        warn!("SSH_STAMP_WIFI_BAND env received but not authenticated; rejecting");
        a.fail()?;
    }
    Ok(())
}

/// Handles `SSH_STAMP_WIFI_STA_SSID` environment variable requests.
///
/// # Errors
/// Returns an error if SSH protocol operations fail or if the SSID is invalid.
pub async fn wifi_sta_ssid_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if *ctx.auth_checked || config_guard.first_login {
        if let Some(s) = env_parser::parse_wifi_station_ssid(a.value()?) {
            notice!(
                ctx.notices,
                "config: wifi station ssid {:?} -> {:?}",
                config_guard.wifi_sta_ssid.as_str(),
                s.as_str()
            );
            config_guard.wifi_sta_ssid = s;
            debug!("Set wifi STATION SSID from ENV");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_STA_SSID invalid and/or too long");
            notice!(
                ctx.notices,
                "SSH_STAMP_WIFI_STA_SSID rejected: empty or over 32 bytes"
            );
            a.fail()?;
        }
    } else {
        warn!("SSH_STAMP_WIFI_STA_SSID env received but not authenticated; rejecting");
        a.fail()?;
    }
    Ok(())
}

/// Handles `SSH_STAMP_WIFI_STA_PSK` environment variable requests.
///
/// # Errors
/// Returns an error if SSH protocol operations fail or if the SSID is invalid.
pub async fn wifi_sta_psk_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if *ctx.auth_checked || config_guard.first_login {
        if let Some(s) = env_parser::parse_wifi_psk(a.value()?) {
            notice!(
                ctx.notices,
                "config: wifi station psk updated ({} chars)",
                s.len()
            );
            config_guard.wifi_sta_pw = s;
            debug!("Set wifi STATION PSK from ENV");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_STA_PSK invalid and/or not within 8-63 characters");
            notice!(
                ctx.notices,
                "SSH_STAMP_WIFI_STA_PSK rejected: must be 8-63 characters"
            );
            a.fail()?;
        }
    } else {
        warn!("SSH_STAMP_WIFI_STA_PSK env received but not authenticated; rejecting");
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
            notice!(
                ctx.notices,
                "config: mac -> {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                mac[0],
                mac[1],
                mac[2],
                mac[3],
                mac[4],
                mac[5]
            );
            config_guard.mac = mac;
            debug!("Set MAC address from ENV: {mac:02X?}");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_MAC_ADDRESS must be XX:XX:XX:XX:XX:XX format");
            notice!(
                ctx.notices,
                "SSH_STAMP_WIFI_MAC_ADDRESS rejected: must be XX:XX:XX:XX:XX:XX"
            );
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
        notice!(ctx.notices, "config: mac -> randomised each boot");
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

/// Handles SSH PTY requests.
///
/// # Errors
///
/// Returns an error if SSH protocol operations fail.
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

/// Rejects SSH exec requests.
///
/// # Errors
///
/// Returns an error if SSH protocol operations fail.
pub fn session_exec(ev: ServEvent<'_, '_>) -> Result<(), sunset::Error> {
    if let ServEvent::SessionExec(a) = ev {
        a.fail()?;
    }
    Ok(())
}

/// Logs why the peer ended the connection.
///
/// Without this the console cannot tell a client saying goodbye from a
/// connection that simply dropped, which is the difference between normal
/// operation and a fault worth chasing.
///
/// # Errors
/// Returns an error if SSH protocol operations fail.
pub fn disconnected(ev: ServEvent<'_, '_>) -> Result<(), sunset::Error> {
    if let ServEvent::Disconnected(d) = ev {
        // Remote-supplied text. It only reaches the local log, but do not
        // pass it anywhere that treats it as trusted.
        let desc = d.desc().unwrap_or("<not valid utf-8>");
        match d.reason() {
            // What OpenSSH sends on a normal exit; not worth an info line.
            Some(DisconnectReason::SSH_DISCONNECT_BY_APPLICATION) => {
                debug!("Client disconnected: {desc}");
            }
            Some(reason) => info!("Client disconnected, {reason:?}: {desc}"),
            None => {
                info!("Client disconnected, reason {}: {desc}", d.reason_code());
            }
        }
    }
    Ok(())
}

/// Returns a `BadUsage` error for unhandled events.
///
/// # Errors
///
/// Always returns `BadUsage` error.
pub fn defunct() -> Result<(), sunset::Error> {
    debug!("Expected caller to handle event");
    sunset::error::BadUsage.fail()
}

/// Handles an SSH client connection, bridging UART and SSH.
///
#[cfg_attr(
    feature = "can",
    doc = "A `can` subsystem channel is bridged concurrently with the shell",
    doc = "(UART) session on the same connection. The whole connection is",
    doc = "torn down when either bridge finishes.",
    doc = ""
)]
/// # Errors
/// Returns an error if SSH protocol operations or I/O fail.
pub async fn ssh_client<'a, 'b, U, P>(
    uart_buff: &'a U,
    ssh_server: &'b SSHServer<'a>,
    chan_pipe: &'b Channel<NoopRawMutex, SessionType, 1>,
    #[cfg_attr(
        not(any(feature = "sftp-ota", feature = "can")),
        allow(unused_variables)
    )]
    platform: &'b P,
    #[cfg(feature = "can")] can_queue: &'b Channel<NoopRawMutex, ChanHandle, 1>,
    notices: &'b NoticeQueue,
    config: &'b SunsetMutex<SSHStampConfig>,
) -> Result<(), sunset::Error>
where
    U: BufferedSerial,
    P: PlatformServices,
{
    debug!("Preparing bridge");
    let mut pending = NoticeDrain::new();
    let session = async {
        let session_type = chan_pipe.receive().await;
        debug!("Checking bridge session type");
        match session_type {
            SessionType::Bridge(ch) => {
                info!("Handling bridge session");
                // stderr, not stdout: stdout is the target's UART, verbatim.
                let (chan_io, mut stderr) = ssh_server.stdio_stderr(ch).await?;
                let (stdin, stdout) = chan_io.split();

                let enabled = notices.lock(|n| n.borrow().enabled());
                if enabled {
                    // Anything queued during session setup (config changes,
                    // rejected requests) had nowhere to go until now.
                    notices.lock(|n| n.borrow_mut().flush_into(&mut pending));
                    if !pending.is_empty() {
                        stderr.write_all(pending.as_bytes()).await?;
                        pending.clear();
                    }
                    let config_guard = config.lock().await;
                    notices::config_summary(&mut stderr, &config_guard).await?;
                    drop(config_guard);
                    notices::emit(&mut stderr, format_args!("bridge connected")).await?;
                }

                info!("Starting bridge");
                let outcome =
                    serial_bridge(stdin, stdout, uart_buff, enabled.then_some(&mut stderr)).await;

                if enabled {
                    // Best effort: the channel is usually already going away,
                    // and failing to say goodbye must not mask `outcome`.
                    let _ = notices::emit(&mut stderr, format_args!("bridge disconnected")).await;
                }
                outcome?;
            }
            #[cfg(feature = "sftp-ota")]
            SessionType::Sftp(ch) => {
                debug!("Handling SFTP session");
                let stdio = ssh_server.stdio(ch).await?;
                let ota_writer = platform.ota_writer();
                ota::run_ota_server::<P::OtaWriter>(stdio, ota_writer).await?;
            }
        }
        Ok(())
    };

    #[cfg(feature = "can")]
    let result = {
        let can_session = async {
            let ch = can_queue.receive().await;
            info!("Handling CAN session");
            let chan_io: ChanInOut<'_> = ssh_server.stdio(ch).await?;
            let (stdin, stdout) = chan_io.split();
            info!("Starting CAN bridge");
            can_bridge(stdin, stdout, platform.can()).await
        };
        match select(session, can_session).await {
            Either::First(r) | Either::Second(r) => r,
        }
    };
    #[cfg(not(feature = "can"))]
    let result = session.await;
    result
}

pub fn bridge_disable() {
    debug!("Bridge disabled: WIP");
}
