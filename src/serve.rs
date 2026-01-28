use core::option::Option::{self, None, Some};
use core::result::Result;
use core::writeln;

use crate::keys;
use crate::pins::PinChannel;

// Embassy
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
// use sunset_async::SunsetMutex;

use heapless::String;
use sunset::{error, ChanHandle, ServEvent, SignKey};
use sunset_async::{ProgressHolder, SSHServer};

use esp_hal::system::software_reset;
use esp_println::{dbg, println};

pub enum SessionType {
    Bridge(ChanHandle),
    //Sftp(ChanHandle),
}

pub async fn connection_loop<'a>(
    serv: &SSHServer<'_>,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
    // pin_channel_ref: &'a SunsetMutex<PinChannel<'a>>,
    // pin_channel_ref: &PinChannel<'a>,
    pin_channel: PinChannel<'a>,
) -> Result<(), sunset::Error> {
    let username = Mutex::<NoopRawMutex, _>::new(String::<20>::new());
    let mut session: Option<ChanHandle> = None;

    println!("Entering connection_loop and prog_loop is next...");

    loop {
        let mut ph = ProgressHolder::new();
        let ev = serv.progress(&mut ph).await?;
        dbg!(&ev);
        #[allow(unreachable_patterns)]
        match ev {
            ServEvent::SessionShell(a) => {
                if let Some(ch) = session.take() {
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
                            // let mut ch = pin_channel_ref.lock().await;
                            // let mut ch: &mut PinChannel<'_>  = pin_channel_ref;
                            let mut ch = pin_channel;
                            if ch.set_tx_pin(pin_num).is_err() {
                                dbg!("Failed to update TX pin");
                            } else {
                                dbg!("TX pin updated");
                            }
                            software_reset();
                        } else {
                            dbg!("Invalid TX pin value");
                        }
                    }
                    "UART_RX_PIN" => {
                        let val = a.value()?;
                        dbg!("Updating UART RX pin to ", val);
                        if let Ok(pin_num) = val.parse::<u8>() {
                            // let mut ch = pin_channel_ref.lock().await;
                            let mut ch = pin_channel;
                            if ch.set_rx_pin(pin_num).is_err() {
                                dbg!("Failed to update RX pin");
                            } else {
                                dbg!("RX pin updated");
                            }
                            software_reset();
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
            _ => (),
        };
    }
}
