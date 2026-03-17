// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use log::{debug, info, trace, warn};

use crate::config::SSHStampConfig;
use crate::espressif::buffered_uart::UART_SIGNAL;
use crate::settings::UART_BUFFER_SIZE;
use crate::store;
use esp_hal::system::software_reset;
use heapless::String;
use storage::flash;

use core::fmt::Debug;
use core::option::Option::{self, None, Some};
use core::result::Result;
use core::sync::atomic::{AtomicBool, Ordering};

// Embassy
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;

// Sunset
use sunset::{ChanHandle, ServEvent, error};
use sunset_async::SunsetMutex;
use sunset_async::{ProgressHolder, SSHServer};

#[derive(Debug)]
pub enum SessionType {
    Bridge(ChanHandle),
    #[cfg(feature = "sftp-ota")]
    Sftp(ChanHandle),
}

static NEEDS_RESET: AtomicBool = AtomicBool::new(false);

pub fn check_and_clear_reset() -> bool {
    NEEDS_RESET.swap(false, Ordering::SeqCst)
}

pub struct ConnectionResult {
    pub needs_reset: bool,
}

pub async fn connection_loop(
    serv: &SSHServer<'_>,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
    config: &SunsetMutex<SSHStampConfig>,
) -> Result<bool, sunset::Error> {
    let mut session: Option<ChanHandle> = None;

    debug!("Entering connection_loop and prog_loop is next...");
    let mut config_changed: bool = false;
    let mut needs_reset: bool = false;

    // Will be set in `ev` PubkeyAuth is accepted and cleared once the channel is sent down chan_pipe
    let mut auth_checked = false;

    loop {
        let mut ph = ProgressHolder::new();
        let ev = serv.progress(&mut ph).await?;

        trace!("{:?}", &ev);
        match ev {
            ServEvent::SessionSubsystem(a) => {
                debug!("ServEvent::SessionSubsystem");

                if !auth_checked {
                    warn!("Unauthenticated SessionSubsystem rejected");
                    a.fail()?;
                    // TODO: Provide a message back to the client and the close the session?
                } else if a.command()?.to_lowercase().as_str() == "sftp" {
                    if let Some(ch) = session.take() {
                        debug_assert!(ch.num() == a.channel());
                        #[cfg(feature = "sftp-ota")]
                        {
                            a.succeed()?;
                            debug!("We got SFTP subsystem");
                            match chan_pipe.try_send(SessionType::Sftp(ch)) {
                                Ok(_) => auth_checked = false,
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
                }
            }
            ServEvent::SessionShell(a) => {
                debug!("ServEvent::SessionShell");

                if !auth_checked {
                    warn!("Unauthenticated SessionShell rejected");
                    a.fail()?;
                } else if let Some(ch) = session.take() {
                    // Save config after connection successful (SessionEnv completed)
                    if config_changed {
                        config_changed = false;
                        let config_guard = config.lock().await;
                        let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
                            panic!("Could not acquire flash storage lock");
                        };
                        let mut flash_storage = flash_storage_guard.lock().await;
                        let _result = store::save(&mut flash_storage, &config_guard).await;
                        drop(config_guard);
                        if needs_reset {
                            needs_reset = false;
                            NEEDS_RESET.store(true, Ordering::SeqCst);
                            debug!("Configuration saved. Device will reset after disconnect.");
                        }
                    }
                    debug_assert!(ch.num() == a.channel());
                    a.succeed()?;
                    debug!("We got shell");
                    UART_SIGNAL.signal(1);
                    debug!("Connection loop: UART_SIGNAL sent");
                    match chan_pipe.try_send(SessionType::Bridge(ch)) {
                        Ok(_) => auth_checked = false,
                        Err(e) => log::error!("Could not send the channel: {:?}", e),
                    };
                } else {
                    a.fail()?;
                }
            }
            ServEvent::FirstAuth(mut a) => {
                debug!("ServEvent::FirstAuth");
                let config_guard = config.lock().await;

                a.enable_password_auth(false)?;

                a.enable_pubkey_auth(true)?;
                if config_guard.first_login {
                    a.allow()?;
                } else {
                    debug!(
                        "FirstAuth received but not first-login, allowing pubkey auth but rejecting 
                        additions of new public keys on already provisioned device"
                    );
                    a.reject()?;
                }
            }
            ServEvent::Hostkeys(h) => {
                debug!("ServEvent::Hostkeys");
                let config_guard = config.lock().await;
                // Just take it from config as private hostkey is generated on first boot.
                h.hostkeys(&[&config_guard.hostkey])?;
            }
            ServEvent::PasswordAuth(a) => {
                warn!("Password auth is not supported, use public key auth instead.");
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
                            a.allow()?;
                            auth_checked = true;
                        } else {
                            debug!("No matching pubkey slot found");
                            a.reject()?;
                        }
                    }
                    _ => {
                        // Only Ed25519 keys supported
                        a.reject()?;
                    }
                }
            }
            ServEvent::OpenSession(a) => {
                debug!("ServEvent::OpenSession");
                match session {
                    Some(_) => {
                        todo!("Can't have two sessions");
                    }
                    None => {
                        // Track the session
                        session = Some(a.accept()?);
                    }
                }
            }
            ServEvent::SessionEnv(a) => {
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
                            config_changed = true;
                            auth_checked = true;
                        } else {
                            warn!("Failed to add new pubkey from ENV");
                            a.fail()?;
                        }
                    }
                    "SSH_STAMP_WIFI_SSID" => {
                        let mut config_guard = config.lock().await;
                        if !(auth_checked || config_guard.first_login) {
                            warn!(
                                "SSH_STAMP_WIFI_SSID env received but not authenticated; rejecting"
                            );
                            a.fail()?;
                        } else {
                            let mut s = String::<32>::new();
                            if s.push_str(a.value()?).is_ok() {
                                config_guard.wifi_ssid = s;
                                debug!("Set wifi SSID from ENV");
                                a.succeed()?;
                                config_changed = true;
                                needs_reset = true;
                            } else {
                                warn!("SSH_STAMP_WIFI_SSID too long");
                                a.fail()?;
                            }
                        }
                    }
                    "SSH_STAMP_WIFI_PSK" => {
                        let mut config_guard = config.lock().await;
                        if !(auth_checked || config_guard.first_login) {
                            warn!(
                                "SSH_STAMP_WIFI_PSK env received but not authenticated; rejecting"
                            );
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
                                    config_changed = true;
                                    needs_reset = true;
                                } else {
                                    warn!("SSH_STAMP_WIFI_PSK push_str failed unexpectedly");
                                    a.fail()?;
                                }
                            }
                        }
                    }
                    _ => {
                        debug!("Ignoring unknown environment variable: {}", a.name()?);
                        a.succeed()?;
                    }
                }
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
                error::BadUsage.fail()?
            }
            ServEvent::PollAgain => {}
        }
    }
}

pub async fn connection_disable() -> () {
    debug!("Connection loop disabled: WIP");
}

pub async fn ssh_wait_for_initialisation<'server>(
    inbuf: &'server mut [u8; UART_BUFFER_SIZE],
    outbuf: &'server mut [u8; UART_BUFFER_SIZE],
) -> SSHServer<'server> {
    SSHServer::new(inbuf, outbuf)
}

pub async fn ssh_disable() -> () {
    debug!("SSH Server disabled: WIP");
    // TODO: Correctly disable/restart SSH Server and/or send messsage to user over SSH
}

use crate::espressif::buffered_uart::BufferedUart;
use crate::serial::serial_bridge;
use sunset_async::ChanInOut;

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
            let (stdin, stdout) = stdio.split();
            info!("Starting bridge");
            serial_bridge(stdin, stdout, uart_buff).await?
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
    };
    Ok(())
}

pub async fn bridge_disable(should_reset: bool) -> () {
    debug!("Bridge disabled");
    if should_reset {
        info!("Configuration changed - resetting device...");
        software_reset();
    }
}
