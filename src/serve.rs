// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::option::Option::{self, None, Some};
use core::result::Result;

use crate::config::SSHStampConfig;
use crate::config_storage;
use crate::keys;
use storage::flash;

// Embassy
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
// use embedded_storage::Storage;
use sunset_async::SunsetMutex;

use heapless::String;
// use sunset::sshwire::SSHEncode;
use sunset::{error, ChanHandle, ServEvent, SignKey};
use sunset_async::{ProgressHolder, SSHServer};

use esp_println::{dbg, println};

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
        let ev = serv.progress(&mut ph).await?;
        dbg!(&ev);
        #[allow(unreachable_patterns)]
        match ev {
            // #[cfg(feature = "sftp-ota")]
            ServEvent::SessionSubsystem(a) => {
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
                        let _result = config_storage::save(&mut flash_storage, &config_guard).await;
                    }
                    debug_assert!(ch.num() == a.channel());
                    a.succeed()?;
                    dbg!("We got shell");
                    let _ = chan_pipe.try_send(SessionType::Bridge(ch));
                } else {
                    a.fail()?;
                }
            }
            ServEvent::FirstAuth(ref a) => {
                // record the username
                if username.lock().await.push_str(a.username()?).is_err() {
                    println!("Too long username")
                }
            }
            ServEvent::Hostkeys(h) => {
                let signkey: SignKey = SignKey::from_openssh(keys::HOST_SECRET_KEY)?;
                h.hostkeys(&[&signkey])?;
            }
            ServEvent::PasswordAuth(a) => {
                a.allow()?;
            }
            ServEvent::PubkeyAuth(a) => {
                a.allow()?;
            }
            ServEvent::OpenSession(a) => {
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
                a.succeed()?;
            }
            ServEvent::SessionExec(a) => {
                a.fail()?;
            }
            ServEvent::Defunct | ServEvent::SessionShell(_) => {
                println!("Expected caller to handle event");
                error::BadUsage.fail()?
            }
            ServEvent::PollAgain => (),
            _ => (),
        }
    }
}
