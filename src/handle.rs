// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! SSH event handlers: authentication, channels, environment variables.
//!
//! Every incoming SSH event is dispatched here by the connection loop in
//! [`serve`](crate::serve). The main entry point is [`session_env`], which
//! routes environment variable requests to handlers like [`pubkey_env`],
//! [`wifi_ap_ssid_env`] and [`wifi_sta_ssid_env`].
//!
//! First-boot provisioning also flows through here: when `first_login` is true,
//! the device accepts any SSH connection (empty password) and allows the
//! client to set `SSH_STAMP_PUBKEY`. Subsequent connections require that key.

use heapless::String;
use log::{debug, info, warn};

use crate::config::SSHStampConfig;
use crate::platform::PlatformServices;
use crate::serial::{BufferedSerial, serial_bridge};

#[cfg(feature = "can")]
use crate::can::can_bridge;

#[cfg(feature = "can")]
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;

use core::result::Result;

use sunset::packets::PubKey;
use sunset::{ChanFail, ChanHandle, ServEvent};
use sunset_async::{ChanInOut, SSHServer, SunsetMutex};

pub mod env_parser {
    use super::String;
    use core::str::FromStr;
    use ssh_stamp_hal::Ipv6Mode;

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

    /// Parses an IPv6 configuration from an environment variable value.
    ///
    /// Accepts `off`, `slaac`, or `<address>/<prefix>` with an optional
    /// `,<gateway>` suffix. The address form is validated by
    /// [`Ipv6Mode::new_static`], which rejects a prefix over 128 and any
    /// address smoltcp would panic on rather than install.
    ///
    /// Returns `None` if the value is not one of those forms.
    #[must_use]
    pub fn parse_ipv6(value: &str) -> Option<Ipv6Mode> {
        if !env_sanitize(value) {
            return None;
        }
        Ipv6Mode::from_str(value).ok()
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

    // Slowest and fastest baud rates accepted for the bridge. The ceiling is
    // what the UART peripherals themselves top out at.
    const UART_BAUD_MIN: u32 = 300;
    const UART_BAUD_MAX: u32 = 5_000_000;

    /// Parses a UART baud rate, accepting 300 to 5000000 baud.
    #[must_use]
    pub fn parse_uart_baud(value: &str) -> Option<u32> {
        let baud: u32 = value.trim().parse().ok()?;
        (UART_BAUD_MIN..=UART_BAUD_MAX)
            .contains(&baud)
            .then_some(baud)
    }

    /// Parses the UART data bits per frame, accepting 5 to 8.
    #[must_use]
    pub fn parse_uart_data_bits(value: &str) -> Option<u8> {
        let bits: u8 = value.trim().parse().ok()?;
        (5..=8).contains(&bits).then_some(bits)
    }

    /// Parses the UART parity setting, accepting `none`, `even` or `odd`.
    #[must_use]
    pub fn parse_uart_parity(value: &str) -> Option<ssh_stamp_hal::Parity> {
        ssh_stamp_hal::Parity::from_str(value.trim()).ok()
    }

    /// Parses the UART stop bits per frame, accepting 1 or 2.
    #[must_use]
    pub fn parse_uart_stop_bits(value: &str) -> Option<u8> {
        let bits: u8 = value.trim().parse().ok()?;
        matches!(bits, 1 | 2).then_some(bits)
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
                    info!("Configuration saved. Rebooting to apply the changes...");
                    platform.reset();
                }
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
) -> Result<(), sunset::Error> {
    if let ServEvent::FirstAuth(mut a) = ev {
        debug!("ServEvent::FirstAuth");
        let config_guard = config.lock().await;

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
            // `SSH_STAMP_WIFI_STA_PSK` is the documented name and matches
            // `SSH_STAMP_WIFI_AP_PSK`; `..._PW` is what the dispatch originally
            // used, so it stays accepted. Sending the documented name used to
            // fall through to the catch-all below, which reports success and
            // drops the value — leaving the station with no password.
            "SSH_STAMP_WIFI_STA_PSK" | "SSH_STAMP_WIFI_STA_PW" => {
                wifi_sta_psk_env(a, config, ctx).await?;
            }
            "SSH_STAMP_WIFI_MAC_ADDRESS" => {
                wifi_mac_address_env(a, config, ctx).await?;
            }
            "SSH_STAMP_WIFI_MAC_RANDOM" => {
                wifi_mac_random_env(a, config, ctx).await?;
            }
            "SSH_STAMP_IPV6" => {
                ipv6_env(a, config, ctx).await?;
            }
            "SSH_STAMP_UART_BAUD" => {
                uart_env(UartParam::Baud, a, config, ctx).await?;
            }
            "SSH_STAMP_UART_DATA_BITS" => {
                uart_env(UartParam::DataBits, a, config, ctx).await?;
            }
            "SSH_STAMP_UART_PARITY" => {
                uart_env(UartParam::Parity, a, config, ctx).await?;
            }
            "SSH_STAMP_UART_STOP_BITS" => {
                uart_env(UartParam::StopBits, a, config, ctx).await?;
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
                a.fail()?;
            }
            Some(trimmed) => {
                if config_guard.add_pubkey(trimmed).is_ok() {
                    debug!("Added new pubkey from ENV");
                    a.succeed()?;
                    if config_guard.first_login {
                        config_guard.first_login = false;
                        *ctx.config_changed = true;
                        *ctx.auth_checked = true;
                    }
                } else {
                    warn!("Failed to add new pubkey from ENV");
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
            config_guard.wifi_ap_ssid = s;
            debug!("Set wifi Access Point SSID from ENV");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_AP_SSID invalid and/or too long");
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
            config_guard.wifi_ap_pw = s;
            debug!("Set WIFI AP PSK from ENV");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_AP_PSK invalid and/or not within 8-63 characters");
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
            config_guard.wifi_ap_band = band;
            debug!("Set WIFI AP band from ENV: {band}");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_BAND must be 2.4g, 5g, or auto");
            a.fail()?;
        }
    } else {
        warn!("SSH_STAMP_WIFI_BAND env received but not authenticated; rejecting");
        a.fail()?;
    }
    Ok(())
}

/// Handles `SSH_STAMP_IPV6` environment variable requests.
///
/// Accepts `off` to drop back to link-local only, `slaac` to autoconfigure
/// from router advertisements (station mode; an access point has no router to
/// solicit), or `<address>/<prefix>[,<gateway>]` for a fixed address — for
/// example `fd00::1/64` on an access point or `2001:db8::2/64,2001:db8::1` on
/// a station. Triggers a config save + reset on success, because the stack is
/// configured once at bring-up.
///
/// # Errors
/// Returns an error if SSH protocol operations fail.
pub async fn ipv6_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if *ctx.auth_checked || config_guard.first_login {
        if let Some(mode) = env_parser::parse_ipv6(a.value()?) {
            config_guard.ipv6 = mode;
            debug!("Set IPv6 mode from ENV: {mode:?}");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_IPV6 must be off, slaac, or <address>/<prefix>[,<gateway>]");
            a.fail()?;
        }
    } else {
        warn!("SSH_STAMP_IPV6 env received but not authenticated; rejecting");
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
            config_guard.wifi_sta_ssid = s;
            debug!("Set wifi STATION SSID from ENV");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_STA_SSID invalid and/or too long");
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
            config_guard.wifi_sta_pw = s;
            debug!("Set wifi STATION PSK from ENV");
            a.succeed()?;
            *ctx.config_changed = true;
            *ctx.needs_reset = true;
        } else {
            warn!("SSH_STAMP_WIFI_STA_PSK invalid and/or not within 8-63 characters");
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

/// A UART line parameter, one per `SSH_STAMP_UART_*` environment variable.
#[derive(Clone, Copy, Debug)]
pub enum UartParam {
    Baud,
    DataBits,
    Parity,
    StopBits,
}

impl UartParam {
    /// The environment variable this parameter is set from.
    const fn env_name(self) -> &'static str {
        match self {
            Self::Baud => "SSH_STAMP_UART_BAUD",
            Self::DataBits => "SSH_STAMP_UART_DATA_BITS",
            Self::Parity => "SSH_STAMP_UART_PARITY",
            Self::StopBits => "SSH_STAMP_UART_STOP_BITS",
        }
    }

    /// The values this parameter accepts, for the rejection log line.
    const fn accepted(self) -> &'static str {
        match self {
            Self::Baud => "300-5000000",
            Self::DataBits => "5-8",
            Self::Parity => "none, even or odd",
            Self::StopBits => "1 or 2",
        }
    }
}

/// Handles the `SSH_STAMP_UART_*` environment variable requests.
///
/// The bridge configures its UART once at boot, so a change is persisted and
/// applied after the reset triggered on the next shell request, like the
/// `WiFi` settings.
///
/// # Errors
/// Returns an error if SSH protocol operations fail.
pub async fn uart_env(
    param: UartParam,
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    ctx: &mut EventContext<'_>,
) -> Result<(), sunset::Error> {
    let mut config_guard = config.lock().await;
    if !(*ctx.auth_checked || config_guard.first_login) {
        warn!(
            "{} env received but not authenticated; rejecting",
            param.env_name()
        );
        return a.fail();
    }

    let value = a.value()?;
    let uart = &mut config_guard.uart_params;
    let applied = match param {
        UartParam::Baud => env_parser::parse_uart_baud(value).map(|v| uart.baud = v),
        UartParam::DataBits => env_parser::parse_uart_data_bits(value).map(|v| uart.data_bits = v),
        UartParam::Parity => env_parser::parse_uart_parity(value).map(|v| uart.parity = v),
        UartParam::StopBits => env_parser::parse_uart_stop_bits(value).map(|v| uart.stop_bits = v),
    };

    if applied.is_some() {
        debug!("Set UART {param:?} from ENV: {uart:?}");
        a.succeed()?;
        *ctx.config_changed = true;
        *ctx.needs_reset = true;
    } else {
        warn!("{} must be {}", param.env_name(), param.accepted());
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
) -> Result<(), sunset::Error>
where
    U: BufferedSerial,
    P: PlatformServices,
{
    debug!("Preparing bridge");
    let session = async {
        let session_type = chan_pipe.receive().await;
        debug!("Checking bridge session type");
        match session_type {
            SessionType::Bridge(ch) => {
                info!("Handling bridge session");
                let chan_io: ChanInOut<'_> = ssh_server.stdio(ch).await?;
                let (stdin, stdout) = chan_io.split();
                info!("Starting bridge");
                serial_bridge(stdin, stdout, uart_buff).await?;
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
