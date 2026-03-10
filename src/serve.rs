// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use log::{debug, info};

use crate::config::SSHStampConfig;
use crate::settings::UART_BUFFER_SIZE;
use crate::store;
use core::option::Option::{self, None, Some};
use core::result::Result;
use storage::flash;
// Embassy
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
// use embedded_storage::Storage;
// use esp_hal::system::software_reset;
use heapless::String;
use sunset_async::SunsetMutex;
// use sunset::sshwire::SSHEncode;
use crate::espressif::buffered_uart::UART_SIGNAL;
use sunset::{ChanHandle, ServEvent, error};
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
    let username = Mutex::<NoopRawMutex, _>::new(String::<20>::new());
    let mut session: Option<ChanHandle> = None;

    debug!("Entering connection_loop and prog_loop is next...");
    let mut config_changed: bool = false;
    loop {
        let mut ph = ProgressHolder::new();
        let ev = serv.progress(&mut ph).await?;
        // debug!(&ev);
        #[allow(unreachable_patterns)]
        match ev {
            // #[cfg(feature = "sftp-ota")]
            ServEvent::SessionSubsystem(a) => {
                info!("ServEvent::SessionSubsystem");
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
                            use log::warn;

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
                if let Some(ch) = session.take() {
                    // Save config after connection successful (SessionEnv completed)
                    if config_changed {
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
            ServEvent::FirstAuth(ref a) => {
                info!("ServEvent::FirstAuth");
                // record the username
                if username.lock().await.push_str(a.username()?).is_err() {
                    info!("Too long username")
                }
            }
            ServEvent::Hostkeys(h) => {
                info!("ServEvent::Hostkeys");
                let config_guard = config.lock().await;
                h.hostkeys(&[&config_guard.hostkey])?;
            }
            ServEvent::PasswordAuth(a) => {
                info!("ServEvent::PasswordAuth");
                a.allow()?;
            }
            ServEvent::PubkeyAuth(a) => {
                info!("ServEvent::PubkeyAuth");
                a.allow()?;
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

                // TODO: Logic to serialise/validate env vars? I.e:
                // a.name.validate(); // Checks the input variable, sanitizes, assigns a target subsystem
                //
                // config.change(c): Apply the config change to the relevant subsystem.
                // i.e: if UART_TX_PIN or UART_RX_PIN, we update the PinChannel with with_channel() to change pins live.
                match a.name()? {
                    "SAVE_CONFIG" => {
                        if a.value()? == "1" {
                            debug!("Triggering config save...");
                            todo!("Implement config save to flash");
                        }
                    }
                    // If the env var is UART_TX_PIN or UART_RX_PIN
                    "UART_TX_PIN" => {
                        let val = a.value()?;
                        debug!("Updating UART TX pin to {}", val);
                        if let Ok(pin_num) = val.parse::<u8>() {
                            let mut config_lock = config.lock().await;
                            config_lock.uart_pins.tx = pin_num;
                            config_changed = true;
                            debug!("TX pin updated");
                        } else {
                            debug!("Invalid TX pin value");
                        }
                    }
                    "UART_RX_PIN" => {
                        let val = a.value()?;
                        debug!("Updating UART RX pin to {}", val);
                        if let Ok(pin_num) = val.parse::<u8>() {
                            let mut config_lock = config.lock().await;
                            config_lock.uart_pins.rx = pin_num;
                            config_changed = true;
                            debug!("RX pin updated");
                        } else {
                            debug!("Invalid RX pin value");
                        }
                    }
                    _ => {
                        debug!("Unknown/unsupported ENV var");
                    }
                }

                // config.save(a): Potentially an optional special environment variable SAVE_CONFIG=1
                // that serialises current config to flash
                // Only save once all ENV requests have been recorded?

                a.succeed()?;
            }
            ServEvent::SessionPty(a) => {
                info!("ServEvent::SessionPty");
                a.succeed()?;
            }
            ServEvent::SessionExec(a) => {
                a.fail()?;
            }
            ServEvent::Defunct | ServEvent::SessionShell(_) => {
                info!("Expected caller to handle event");
                error::BadUsage.fail()?
            }
            ServEvent::PollAgain => {
                // info!("ServEvent::PollAgain");
            }
            _ => (),
        }
    }
}

pub async fn connection_disable() -> () {
    // disable connection loop
    info!("Connection loop disabled");
    // TODO: Correctly disable/restart Conection loop and/or send messsage to user over SSH
    // software_reset();
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
    // software_reset();
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
            let (stdin, stdout) = stdio.split();
            info!("Starting bridge");
            serial_bridge(stdin, stdout, uart_buff).await?
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
    // software_reset();
}
