// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use log::{debug, info, warn};

use crate::config::SSHStampConfig;
use crate::espressif::buffered_uart::UART_SIGNAL;
use crate::settings::{PUBKEY_AUTH, UART_BUFFER_SIZE};
use crate::store;
use storage::flash;

use core::option::Option::{self, None, Some};
use core::result::Result;

// Embassy
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use esp_hal::system::software_reset;

// Sunset
use sunset::{ChanHandle, ServEvent, error};
use sunset_async::SunsetMutex;
use sunset_async::{ProgressHolder, SSHServer};

pub enum SessionType {
    Bridge(ChanHandle),
    #[cfg(feature = "sftp-ota")]
    Sftp(ChanHandle),
}

pub async fn connection_loop(
    serv: &SSHServer<'_>,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
    config: &SunsetMutex<SSHStampConfig>,
) -> Result<(), sunset::Error> {
    //let username = Mutex::<NoopRawMutex, _>::new(String::<20>::new());
    let mut session: Option<ChanHandle> = None;

    debug!("Entering connection_loop and prog_loop is next...");
    let mut config_changed: bool = false;
    loop {
        let mut ph = ProgressHolder::new();
        let ev = serv.progress(&mut ph).await?;
        // debug!(&ev);
        match ev {
            // #[cfg(feature = "sftp-ota")]
            ServEvent::SessionSubsystem(a) => {
                info!("ServEvent::SessionSubsystem");
                {
                    let config_guard = config.lock().await;
                    if config_guard.first_boot {
                        warn!("Unauthenticated SessionSubsystem rejected");
                        a.fail()?;
                        // TODO: Handle this gracefully
                        // TODO: Provide a message back to the client and the close the session?
                        software_reset();
                    }
                }
                if a.command()?.to_lowercase().as_str() == "sftp" {
                    if let Some(ch) = session.take() {
                        debug_assert!(ch.num() == a.channel());
                        #[cfg(feature = "sftp-ota")]
                        {
                            a.succeed()?;
                            info!("We got SFTP subsystem");
                            let _ = chan_pipe.try_send(SessionType::Sftp(ch));
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
            ServEvent::SessionShell(a) => {
                info!("ServEvent::SessionShell");
                {
                    let config_guard = config.lock().await;
                    if config_guard.first_boot {
                        warn!("Unauthenticated SessionShell rejected");
                        a.fail()?;
                        // TODO: Handle this gracefully
                        // TODO: Provide a message back to the client and the close the session?
                        software_reset();
                    }
                }
                if let Some(ch) = session.take() {
                    // Save config after connection successful (SessionEnv completed)
                    if config_changed {
                        config_changed = false; // TODO: Avoid unnecessary "does not neet to be mutable" warnings for now
                        let config_guard = config.lock().await;
                        let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
                            panic!("Could not acquire flash storage lock");
                        };
                        let mut flash_storage = flash_storage_guard.lock().await;
                        // TODO: Migrate this function/test to embedded-test.
                        // Quick roundtrip test for SSHStampConfig
                        // ssh_stamp::config::roundtrip_config();
                        let _result = store::save(&mut flash_storage, &config_guard).await;
                    }
                    debug_assert!(ch.num() == a.channel());
                    a.succeed()?;
                    info!("We got shell");
                    // Signal for uart task to configure pins and run. Value is irrelevant.
                    UART_SIGNAL.signal(1);
                    info!("Connection loop: UART_SIGNAL sent");
                    let _ = chan_pipe.try_send(SessionType::Bridge(ch));
                } else {
                    a.fail()?;
                }
            }
            ServEvent::FirstAuth(mut a) => {
                info!("ServEvent::FirstAuth");
                // Allow the "first auth" behaviour only on first-boot-like configs.
                // Consider the device in first-boot state when there is no password
                // and no stored client pubkeys.
                let config_guard = config.lock().await;

                // Disable password auth method regardless.
                a.enable_password_auth(false)?;
                if config_guard.first_boot {
                    // SECURITY: We have no users; enable pubkey auth so the
                    // provisioner can add a key.
                    a.enable_pubkey_auth(PUBKEY_AUTH)?;
                    a.allow()?; // SECURITY: Controversial (but necessary to provision?)
                } else {
                    // Not first boot: do not auto-allow; reject the first-auth helper.
                    info!(
                        "FirstAuth received but not first-boot, allowing pubkey auth but rejecting 
                        additions of new public keys on already provisioned device"
                    );
                    a.enable_pubkey_auth(PUBKEY_AUTH)?;
                    a.reject()?;
                }
            }
            ServEvent::Hostkeys(h) => {
                info!("ServEvent::Hostkeys");
                let config_guard = config.lock().await;
                // Just take it from config as private hostkey is generated on first boot.
                h.hostkeys(&[&config_guard.hostkey])?;
            }
            ServEvent::PasswordAuth(a) => {
                // Password auth is not supported; always reject.
                a.reject()?;
            }
            ServEvent::PubkeyAuth(a) => {
                info!("ServEvent::PubkeyAuth");
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
                        } else {
                            info!("No matching pubkey slot found");
                            a.reject()?;
                            software_reset(); // TODO: Handle better HSM-flow-wise.
                        }
                    }
                    _ => {
                        // Only Ed25519 keys supported
                        a.reject()?;
                        software_reset(); // TODO: Handle better HSM-flow-wise.
                    }
                }
            }
            ServEvent::OpenSession(a) => {
                info!("ServEvent::OpenSession");
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
                        // Only allow adding a pubkey via ENV on first-boot-like configs.

                        if !config_guard.first_boot {
                            warn!("SSH_STAMP_PUBKEY env received but not first-boot; rejecting");
                            a.fail()?;
                            break Ok(()); // TODO: Do better HSM-flow-wise
                        } else if config_guard.add_pubkey(a.value()?).is_ok() {
                            info!("Added new pubkey from ENV");
                            a.succeed()?;
                            // Mark that config has changed and clear first_boot so
                            // future connections are not treated as first-boot.
                            config_guard.first_boot = false;
                            config_changed = true;
                            // Don't immediately allow the new user/key to establish bridge but reboot first?
                            //software_reset(); // TODO: Do better HSM-flow-wise.
                        } else {
                            warn!("Failed to add new pubkey from ENV");
                            a.fail()?;
                        }
                    }
                    _ => {
                        warn!("Unsupported environment variable");
                        a.fail()?;
                    }
                }

                // config.save(a): Potentially an optional special environment variable SAVE_CONFIG=1
                // that serialises current config to flash
                // Only save once all ENV requests have been recorded?

                //a.succeed()?;
            }
            ServEvent::SessionPty(a) => {
                info!("ServEvent::SessionPty");
                a.succeed()?;
            }
            ServEvent::SessionExec(a) => {
                a.fail()?;
            }
            ServEvent::Defunct => {
                info!("Expected caller to handle event");
                error::BadUsage.fail()?
            }
            ServEvent::PollAgain => {
                // info!("ServEvent::PollAgain");
            }
        }
    }
}

pub async fn connection_disable() -> () {
    // disable connection loop
    info!("Connection loop disabled");
    // TODO: Correctly disable/restart Conection loop and/or send messsage to user over SSH
    software_reset();
}

pub async fn ssh_wait_for_initialisation<'server>(
    inbuf: &'server mut [u8; UART_BUFFER_SIZE],
    outbuf: &'server mut [u8; UART_BUFFER_SIZE],
) -> SSHServer<'server> {
    SSHServer::new(inbuf, outbuf)
}

pub async fn ssh_disable() -> () {
    // drop ssh server
    info!("SSH Server disabled");
    // TODO: Correctly disable/restart SSH Server and/or send messsage to user over SSH
    software_reset();
}

use crate::espressif::buffered_uart::BufferedUart;
use crate::serial::serial_bridge;
use sunset_async::ChanInOut;

pub async fn handle_ssh_client<'a, 'b>(
    uart_buff: &'a BufferedUart,
    ssh_server: &'b SSHServer<'a>,
    chan_pipe: &'b Channel<NoopRawMutex, SessionType, 1>,
) -> Result<(), sunset::Error> {
    info!("Preparing bridge");
    let session_type = chan_pipe.receive().await;
    info!("Checking bridge session type");
    match session_type {
        SessionType::Bridge(ch) => {
            info!("Handling bridge session");
            let stdio: ChanInOut<'_> = ssh_server.stdio(ch).await?;
            let stdio2 = stdio.clone();
            info!("Starting bridge");
            serial_bridge(stdio, stdio2, uart_buff).await?
        }
        #[cfg(feature = "sftp-ota")]
        SessionType::Sftp(ch) => {
            {
                info!("Handling SFTP session");
                let stdio = ssh_server.stdio(ch).await?;
                // TODO: Use a configuration flag to select the hardware specific OtaActions implementer
                let ota_writer = storage::esp_ota::OtaWriter::new();
                ota::run_ota_server::<storage::esp_ota::OtaWriter>(stdio, ota_writer).await?
            }
        }
    };
    Ok(())
}

pub async fn bridge_disable() -> () {
    // disable bridge
    info!("Bridge disabled");
    // TODO: Correctly disable/restart bridge and/or send message to user over SSH
    software_reset();
}
