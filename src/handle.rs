// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use log::{debug, info, warn};

use crate::config::SSHStampConfig;
use crate::espressif::buffered_uart::UART_SIGNAL;
use crate::serial::serial_bridge;
use crate::store;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use esp_hal::system::software_reset;
use hal_espressif::flash;
use heapless::String;

use core::fmt::Debug;
use core::option::Option::None;
use core::result::Result;

use sunset::{ChanHandle, ServEvent};
use sunset_async::{ChanInOut, SSHServer, SunsetMutex};

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
                "FirstAuth received but not first-login, allowing pubkey auth but rejecting \
                additions of new public keys on already provisioned device"
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
                let mut config_guard = config.lock().await;

                if !config_guard.first_login {
                    warn!("SSH_STAMP_PUBKEY env received but not first-login; rejecting");
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
            }
            "SSH_STAMP_WIFI_SSID" => {
                let mut config_guard = config.lock().await;
                if !(*ctx.auth_checked || config_guard.first_login) {
                    warn!("SSH_STAMP_WIFI_SSID env received but not authenticated; rejecting");
                    a.fail()?;
                } else {
                    let mut s = String::<32>::new();
                    if s.push_str(a.value()?).is_ok() {
                        config_guard.wifi_ssid = s;
                        debug!("Set wifi SSID from ENV");
                        a.succeed()?;
                        *ctx.config_changed = true;
                        *ctx.needs_reset = true;
                    } else {
                        warn!("SSH_STAMP_WIFI_SSID too long");
                        a.fail()?;
                    }
                }
            }
            "SSH_STAMP_WIFI_PSK" => {
                let mut config_guard = config.lock().await;
                if !(*ctx.auth_checked || config_guard.first_login) {
                    warn!("SSH_STAMP_WIFI_PSK env received but not authenticated; rejecting");
                    a.fail()?;
                } else {
                    let value = a.value()?;
                    if value.len() < 8 {
                        warn!("SSH_STAMP_WIFI_PSK too short (min 8 characters)");
                        a.fail()?;
                    } else if value.len() > 63 {
                        warn!("SSH_STAMP_WIFI_PSK too long (max 63 characters)");
                        a.fail()?;
                    } else {
                        let mut s = String::<63>::new();
                        if s.push_str(value).is_ok() {
                            config_guard.wifi_pw = Some(s);
                            debug!("Set WIFI PSK from ENV");
                            a.succeed()?;
                            *ctx.config_changed = true;
                            *ctx.needs_reset = true;
                        } else {
                            warn!("SSH_STAMP_WIFI_PSK push_str failed unexpectedly");
                            a.fail()?;
                        }
                    }
                }
            }
            "SSH_STAMP_WIFI_MAC_ADDRESS" => {
                let mut config_guard = config.lock().await;
                if !(*ctx.auth_checked || config_guard.first_login) {
                    warn!(
                        "SSH_STAMP_WIFI_MAC_ADDRESS env received but not authenticated; rejecting"
                    );
                    a.fail()?;
                } else {
                    let value = a.value()?;
                    if value.len() != 17 {
                        warn!("SSH_STAMP_WIFI_MAC_ADDRESS must be XX:XX:XX:XX:XX:XX format");
                        a.fail()?;
                    } else {
                        let parts: heapless::Vec<u8, 6> = value
                            .split(':')
                            .filter_map(|p| u8::from_str_radix(p, 16).ok())
                            .collect();
                        if parts.len() == 6 {
                            let mac: [u8; 6] =
                                [parts[0], parts[1], parts[2], parts[3], parts[4], parts[5]];
                            config_guard.mac = mac;
                            debug!("Set MAC address from ENV: {:02X?}", mac);
                            a.succeed()?;
                            *ctx.config_changed = true;
                            *ctx.needs_reset = true;
                        } else {
                            warn!("SSH_STAMP_WIFI_MAC_ADDRESS invalid format");
                            a.fail()?;
                        }
                    }
                }
            }
            "SSH_STAMP_WIFI_MAC_RANDOM" => {
                let mut config_guard = config.lock().await;
                if !(*ctx.auth_checked || config_guard.first_login) {
                    warn!(
                        "SSH_STAMP_WIFI_MAC_RANDOM env received but not authenticated; rejecting"
                    );
                    a.fail()?;
                } else {
                    config_guard.mac = [0xFF; 6];
                    debug!("Set MAC address to random mode");
                    a.succeed()?;
                    *ctx.config_changed = true;
                    *ctx.needs_reset = true;
                }
            }
            _ => {
                debug!("Ignoring unknown environment variable: {}", a.name()?);
                a.succeed()?;
            }
        }
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
