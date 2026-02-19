// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::option::Option::{self, None, Some};
use core::result::Result;

use crate::config::SSHStampConfig;
use crate::keys;
use crate::store;
use storage::flash;

// Embassy
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
// use embedded_storage::Storage;
use esp_hal::system::software_reset;
use heapless::String;
use sunset_async::SunsetMutex;
// use sunset::sshwire::SSHEncode;
use crate::espressif::buffered_uart::UART_SIGNAL;
use esp_println::{dbg, println};
use sunset::{ChanHandle, ServEvent, SignKey, error};
use sunset_async::{ProgressHolder, SSHServer};

pub enum SessionType {
    Bridge(ChanHandle),
    Sftp(ChanHandle),
}

pub async fn connection_loop<'a>(
    serv: &SSHServer<'_>,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
    config: &'a SunsetMutex<SSHStampConfig>,
) -> Result<(), sunset::Error> {
    let username = Mutex::<NoopRawMutex, _>::new(String::<20>::new());
    let mut session: Option<ChanHandle> = None;

    println!("Entering connection_loop and prog_loop is next...");
    let mut config_changed: bool = false;
    loop {
        let mut ph = ProgressHolder::new();
        // dbg!("Waiting for ssh server event");
        let ev = serv.progress(&mut ph).await?;
        // dbg!(&ev);
        #[allow(unreachable_patterns)]
        match ev {
            // #[cfg(feature = "sftp-ota")]
            ServEvent::SessionSubsystem(a) => {
                println!("ServEvent::SessionSubsystem");
                if a.command()?.to_lowercase().as_str() == "sftp" {
                    if let Some(ch) = session.take() {
                        debug_assert!(ch.num() == a.channel());

                        a.succeed()?;
                        dbg!("We got SFTP subsystem");
                        let _ = chan_pipe.try_send(SessionType::Sftp(ch));
                    } else {
                        a.fail()?;
                    }
                } else {
                    a.fail()?;
                }
            }
            ServEvent::SessionShell(a) => {
                println!("ServEvent::SessionShell");
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
                    dbg!("We got shell");
                    // Signal for uart task to configure pins and run. Value is irrelevant.
                    UART_SIGNAL.signal(1);
                    println!("Connection loop: UART_SIGNAL sent");
                    let _ = chan_pipe.try_send(SessionType::Bridge(ch));
                } else {
                    a.fail()?;
                }
            }
            ServEvent::FirstAuth(ref a) => {
                println!("ServEvent::FirstAuth");
                // record the username
                if username.lock().await.push_str(a.username()?).is_err() {
                    println!("Too long username")
                }
            }
            ServEvent::Hostkeys(h) => {
                println!("ServEvent::Hostkeys");
                let signkey: SignKey = SignKey::from_openssh(keys::HOST_SECRET_KEY)?;
                h.hostkeys(&[&signkey])?;
            }
            ServEvent::PasswordAuth(a) => {
                println!("ServEvent::PasswordAuth");
                a.allow()?;
            }
            ServEvent::PubkeyAuth(a) => {
                println!("ServEvent::PubkeyAuth");
                a.allow()?;
            }
            ServEvent::OpenSession(a) => {
                println!("ServEvent::OpenSession");
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
                dbg!("Got ENV request");
                dbg!(a.name()?);
                dbg!(a.value()?);

                // TODO: Logic to serialise/validate env vars? I.e:
                // a.name.validate(); // Checks the input variable, sanitizes, assigns a target subsystem
                //
                // config.change(c): Apply the config change to the relevant subsystem.
                // i.e: if UART_TX_PIN or UART_RX_PIN, we update the PinChannel with with_channel() to change pins live.
                match a.name()? {
                    "SAVE_CONFIG" => {
                        if a.value()? == "1" {
                            dbg!("Triggering config save...");
                            todo!("Implement config save to flash");
                        }
                    }
                    // If the env var is UART_TX_PIN or UART_RX_PIN
                    "UART_TX_PIN" => {
                        let val = a.value()?;
                        dbg!("Updating UART TX pin to ", val);
                        if let Ok(pin_num) = val.parse::<u8>() {
                            let mut config_lock = config.lock().await;
                            config_lock.uart_pins.tx = pin_num;
                            config_changed = true;
                            dbg!("TX pin updated");
                        } else {
                            dbg!("Invalid TX pin value");
                        }
                    }
                    "UART_RX_PIN" => {
                        let val = a.value()?;
                        dbg!("Updating UART RX pin to ", val);
                        if let Ok(pin_num) = val.parse::<u8>() {
                            let mut config_lock = config.lock().await;
                            config_lock.uart_pins.rx = pin_num;
                            config_changed = true;
                            dbg!("RX pin updated");
                        } else {
                            dbg!("Invalid RX pin value");
                        }
                    }
                    _ => {
                        dbg!("Unknown/unsupported ENV var");
                    }
                }

                // config.save(a): Potentially an optional special environment variable SAVE_CONFIG=1
                // that serialises current config to flash
                // Only save once all ENV requests have been recorded?

                a.succeed()?;
            }
            ServEvent::SessionPty(a) => {
                println!("ServEvent::SessionPty");
                a.succeed()?;
            }
            ServEvent::SessionExec(a) => {
                a.fail()?;
            }
            ServEvent::Defunct | ServEvent::SessionShell(_) => {
                println!("Expected caller to handle event");
                error::BadUsage.fail()?
            }
            ServEvent::PollAgain => {
                // println!("ServEvent::PollAgain");
            }
            _ => (),
        }
    }
}

pub async fn connection_disable() -> () {
    // disable connection loop
    software_reset();
}

pub async fn ssh_wait_for_initialisation<'server>(
    inbuf: &'server mut [u8; crate::espressif::buffered_uart::UART_BUFFER_SIZE],
    outbuf: &'server mut [u8; crate::espressif::buffered_uart::UART_BUFFER_SIZE],
) -> SSHServer<'server> {
    let ssh_server = SSHServer::new(inbuf, outbuf);
    ssh_server
}

pub async fn ssh_disable() -> () {
    // drop ssh server
    software_reset();
}

// use crate::serve::SessionType;
use crate::espressif::buffered_uart::BufferedUart;
use crate::serial::serial_bridge;
use sunset_async::ChanInOut;

pub async fn handle_ssh_client<'a, 'b>(
    uart_buff: &'a BufferedUart,
    ssh_server: &'b SSHServer<'a>,
    chan_pipe: &'b Channel<NoopRawMutex, SessionType, 1>,
) -> Result<(), sunset::Error> {
    dbg!("Preparing bridge");
    let bridge = {
        let chan_pipe = chan_pipe;
        let session_type = chan_pipe.receive().await;
        dbg!("Checking bridge session type");
        match session_type {
            SessionType::Bridge(ch) => {
                dbg!("Handling bridge session");
                let stdio: ChanInOut<'_> = ssh_server.stdio(ch).await?;
                let stdio2 = stdio.clone();
                dbg!("Starting bridge");
                serial_bridge(stdio, stdio2, uart_buff).await?
            }
            SessionType::Sftp(_ch) => {
                dbg!("Handling SFTP session");
                // Handle SFTP session
                //     todo!()
            }
        };
        Ok(())
    };
    bridge
}

pub async fn bridge_disable() -> () {
    // disable bridge
    software_reset();
}
