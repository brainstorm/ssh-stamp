// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use log::{debug, info, trace, warn};

use crate::config::SSHStampConfig;
use crate::espressif::buffered_uart::UART_SIGNAL;
use crate::settings::UART_BUFFER_SIZE;
use crate::store;
use esp_hal::system::software_reset;
use storage::flash;

use core::fmt::Debug;
use core::option::Option::{self, None, Some};
use core::result::Result;

// Embassy
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;

// Sunset
use sunset::event::ServPubkeyAuth;
use sunset::{ChanFail, ChanHandle, ServEvent, error};
use sunset_async::SunsetMutex;
use sunset_async::{ProgressHolder, SSHServer};

mod env_parser {
    use heapless::String;

    pub fn parse_wifi_ssid(value: &str) -> Option<String<32>> {
        let mut s = String::new();
        s.push_str(value).ok()?;
        Some(s)
    }

    pub fn parse_wifi_psk(value: &str) -> Option<String<63>> {
        if value.len() < 8 || value.len() > 63 {
            return None;
        }
        let mut s = String::new();
        s.push_str(value).ok()?;
        Some(s)
    }

    pub fn parse_mac_address(value: &str) -> Option<[u8; 6]> {
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

async fn save_config_and_reboot(
    config: &SunsetMutex<SSHStampConfig>,
    config_changed: bool,
    needs_reset: bool,
) {
    if !config_changed {
        return;
    }

    let config_guard = config.lock().await;
    let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
        panic!("Could not acquire flash storage lock");
    };
    let mut flash_storage = flash_storage_guard.lock().await;
    let _result = store::save(&mut flash_storage, &config_guard);
    drop(config_guard);

    if needs_reset {
        info!("Configuration saved. Rebooting to apply WiFi changes...");
        software_reset();
    }
}

/// Handles the SSH connection loop, processing events from clients.
///
/// # Errors
/// Returns an error if SSH protocol operations fail.
///
/// # Panics
/// Panics if flash storage lock cannot be acquired when saving configuration.
pub async fn connection_loop(
    serv: &SSHServer<'_>,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
    config: &SunsetMutex<SSHStampConfig>,
) -> Result<(), sunset::Error> {
    let mut session: Option<ChanHandle> = None;
    let mut config_changed = false;
    let mut needs_reset = false;
    let mut auth_checked = false;

    loop {
        let mut ph = ProgressHolder::new();
        let ev = serv.progress(&mut ph).await?;

        trace!("{:?}", &ev);
        match ev {
            ServEvent::SessionSubsystem(a) => {
                handle_session_subsystem(a, &mut session, &mut auth_checked, chan_pipe)?;
            }
            ServEvent::SessionShell(a) => {
                handle_session_shell(
                    a,
                    &mut session,
                    &mut config_changed,
                    needs_reset,
                    config,
                    chan_pipe,
                    &mut auth_checked,
                )
                .await;
            }
            ServEvent::FirstAuth(mut a) => {
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
            ServEvent::Hostkeys(h) => {
                debug!("ServEvent::Hostkeys");
                let config_guard = config.lock().await;
                h.hostkeys(&[&config_guard.hostkey])?;
            }
            ServEvent::PasswordAuth(a) => {
                warn!("Password auth not supported, use public key auth");
                a.reject()?;
            }
            ServEvent::PubkeyAuth(a) => {
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
                            auth_checked = true;
                            a.allow()?;
                        } else {
                            debug!("No matching pubkey slot found");
                            a.reject()?;
                        }
                    }
                    sunset::packets::PubKey::Unknown(_) => {
                        a.reject()?;
                    }
                }
            }
            ServEvent::OpenSession(a) => {
                debug!("ServEvent::OpenSession");
                match session {
                    Some(_) => {
                        warn!("Rejecting duplicate session channel");
                        a.reject(ChanFail::SSH_OPEN_ADMINISTRATIVELY_PROHIBITED)?;
                    }
                    None => {
                        // Track the session
                        session = Some(a.accept()?);
                    }
                }
            }
            ServEvent::SessionEnv(a) => {
                handle_session_env(
                    a,
                    config,
                    &mut config_changed,
                    &mut needs_reset,
                    &mut auth_checked,
                )
                .await?;
            }
            ServEvent::SessionPty(a) => {
                let first_login = { config.lock().await.first_login };
                if auth_checked || first_login {
                    debug!("ServEvent::SessionPty: Session granted");
                    a.succeed()?;
                } else {
                    debug!("ServEvent::SessionPty: No auth not session");
                    a.fail()?;
                }
            }
            ServEvent::SessionExec(a) => {
                a.fail()?;
            }
            ServEvent::Defunct => {
                debug!("Expected caller to handle event");
                error::BadUsage.fail()?;
            }
            ServEvent::PollAgain => {}
        }
    }
}

#[cfg(feature = "sftp-ota")]
fn handle_session_subsystem(
    a: sunset::event::ServExecRequest<'_, '_>,
    session: &mut Option<ChanHandle>,
    auth_checked: &mut bool,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
) -> Result<(), sunset::Error> {
    debug!("ServEvent::SessionSubsystem");
    if !*auth_checked {
        warn!("Unauthenticated SessionSubsystem rejected");
        a.fail()?;
        return Ok(());
    }

    if a.command()?.to_lowercase().as_str() != "sftp" {
        return Ok(());
    }

    if let Some(ch) = session.take() {
        debug_assert!(ch.num() == a.channel());
        a.succeed()?;
        debug!("We got SFTP subsystem");
        match chan_pipe.try_send(SessionType::Sftp(ch)) {
            Ok(_) => *auth_checked = false,
            Err(e) => log::error!("Could not send the channel: {:?}", e),
        }
    } else {
        a.fail()?;
    }
    Ok(())
}

#[cfg(not(feature = "sftp-ota"))]
fn handle_session_subsystem(
    a: sunset::event::ServExecRequest<'_, '_>,
    session: &mut Option<ChanHandle>,
    auth_checked: &mut bool,
    _chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
) -> Result<(), sunset::Error> {
    debug!("ServEvent::SessionSubsystem");
    if !*auth_checked {
        warn!("Unauthenticated SessionSubsystem rejected");
        a.fail()?;
        return Ok(());
    }

    if a.command()?.to_lowercase().as_str() != "sftp" {
        return Ok(());
    }

    if session.take().is_some() {
        warn!("SFTP subsystem requested but not supported in this build");
        a.fail()?;
    } else {
        a.fail()?;
    }
    Ok(())
}

async fn handle_session_shell(
    a: sunset::event::ServShellRequest<'_, '_>,
    session: &mut Option<ChanHandle>,
    config_changed: &mut bool,
    needs_reset: bool,
    config: &SunsetMutex<SSHStampConfig>,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
    auth_checked: &mut bool,
) {
    debug!("ServEvent::SessionShell");
    if !*auth_checked {
        warn!("Unauthenticated SessionShell rejected");
        let _ = a.fail();
        return;
    }

    if let Some(ch) = session.take() {
        let cc = *config_changed;
        *config_changed = false;
        save_config_and_reboot(config, cc, needs_reset).await;
        debug_assert!(ch.num() == a.channel());
        let _ = a.succeed();
        UART_SIGNAL.signal(1);
        match chan_pipe.try_send(SessionType::Bridge(ch)) {
            Ok(()) => *auth_checked = false,
            Err(e) => log::error!("Could not send the channel: {e:?}"),
        }
    } else {
        let _ = a.fail();
    }
}

async fn handle_session_env(
    a: sunset::event::ServEnvironmentRequest<'_, '_>,
    config: &SunsetMutex<SSHStampConfig>,
    config_changed: &mut bool,
    needs_reset: &mut bool,
    auth_checked: &mut bool,
) -> Result<(), sunset::Error> {
    debug!("Got ENV request");
    debug!("ENV name: {}", a.name()?);
    debug!("ENV value: {}", a.value()?);

    match a.name()? {
        "LANG" => {
            // Ignore, but succeed to avoid client-side warnings
            // This env variable will always be sent by OpenSSH client.
            a.succeed()?;
        }
        "SSH_STAMP_PUBKEY" => {
            let mut config_guard = config.lock().await;

            if !config_guard.first_login {
                warn!("SSH_STAMP_PUBKEY env received but not first-login; rejecting");
                a.fail()?;
            } else if config_guard.add_pubkey(a.value()?).is_ok() {
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
        "SSH_STAMP_WIFI_SSID" => {
            let mut config_guard = config.lock().await;
            if *auth_checked || config_guard.first_login {
                if let Some(s) = env_parser::parse_wifi_ssid(a.value()?) {
                    config_guard.wifi_ssid = s;
                    debug!("Set wifi SSID from ENV");
                    a.succeed()?;
                    *config_changed = true;
                    *needs_reset = true;
                } else {
                    warn!("SSH_STAMP_WIFI_SSID too long");
                    a.fail()?;
                }
            } else {
                warn!("SSH_STAMP_WIFI_SSID env received but not authenticated; rejecting");
                a.fail()?;
            }
        }
        "SSH_STAMP_WIFI_PSK" => {
            let mut config_guard = config.lock().await;
            if *auth_checked || config_guard.first_login {
                if let Some(s) = env_parser::parse_wifi_psk(a.value()?) {
                    config_guard.wifi_pw = Some(s);
                    debug!("Set WIFI PSK from ENV");
                    a.succeed()?;
                    *config_changed = true;
                    *needs_reset = true;
                } else {
                    warn!("SSH_STAMP_WIFI_PSK must be 8-63 characters");
                    a.fail()?;
                }
            } else {
                warn!("SSH_STAMP_WIFI_PSK env received but not authenticated; rejecting");
                a.fail()?;
            }
        }
        "SSH_STAMP_WIFI_MAC_ADDRESS" => {
            let mut config_guard = config.lock().await;
            if *auth_checked || config_guard.first_login {
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
        }
        "SSH_STAMP_WIFI_MAC_RANDOM" => {
            let mut config_guard = config.lock().await;
            if *auth_checked || config_guard.first_login {
                config_guard.mac = [0xFF; 6];
                debug!("Set MAC address to random mode");
                a.succeed()?;
                *config_changed = true;
                *needs_reset = true;
            } else {
                warn!("SSH_STAMP_WIFI_MAC_RANDOM env received but not authenticated; rejecting");
                a.fail()?;
            }
        }
        _ => {
            debug!("Ignoring unknown environment variable: {}", a.name()?);
            a.succeed()?;
        }
    }
    Ok(())
}

pub fn connection_disable() {
    debug!("Connection loop disabled: WIP");
    // TODO: Correctly disable/restart Conection loop and/or send messsage to user over SSH
}

pub fn ssh_wait_for_initialisation<'server>(
    inbuf: &'server mut [u8; UART_BUFFER_SIZE],
    outbuf: &'server mut [u8; UART_BUFFER_SIZE],
) -> SSHServer<'server> {
    SSHServer::new(inbuf, outbuf)
}

pub fn ssh_disable() {
    debug!("SSH Server disabled: WIP");
    // TODO: Correctly disable/restart SSH Server and/or send messsage to user over SSH
}

use crate::espressif::buffered_uart::BufferedUart;
use crate::serial::serial_bridge;
use sunset_async::ChanInOut;

/// Handles an SSH client connection, bridging UART and SSH.
///
/// # Errors
/// Returns an error if SSH protocol operations or I/O fail.
pub async fn handle_ssh_client<'a, 'b>(
    uart_buff: &'a BufferedUart,
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
            let (input, output) = stdio.split();
            info!("Starting bridge");
            serial_bridge(input, output, uart_buff).await?;
        }
        #[cfg(feature = "sftp-ota")]
        SessionType::Sftp(ch) => {
            {
                debug!("Handling SFTP session");
                let stdio = ssh_server.stdio(ch).await?;
                // TODO: Use a configuration flag to select the hardware specific OtaActions implementer
                let ota_writer = storage::esp_ota::OtaWriter::new();
                ota::run_ota_server::<storage::esp_ota::OtaWriter>(stdio, ota_writer).await?
            }
        }
    }
    Ok(())
}

pub fn bridge_disable() {
    // disable bridge
    debug!("Bridge disabled: WIP");
    // TODO: Correctly disable/restart bridge and/or send message to user over SSH
    // software_reset();
}
