use core::option::Option::{self, None, Some};
use core::result::Result;

use crate::pins::PinChannel;
use crate::espressif::buffered_uart::BufferedUart;
use crate::keys;
use crate::serial::serial_bridge;

// Embassy
use embassy_futures::select::{select3, Either3};
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use sunset_async::SunsetMutex;

use heapless::String;
use sunset::{error, ChanHandle, ServEvent, SignKey};
use sunset_async::{ProgressHolder, SSHServer};

use esp_println::{dbg, println};

enum SessionType {
    Bridge(ChanHandle),
    //Sftp(ChanHandle),
}

async fn connection_loop(
    serv: &SSHServer<'_>,
    chan_pipe: &Channel<NoopRawMutex, SessionType, 1>,
    pin_channel_ref: &'static SunsetMutex<PinChannel>,
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
                            let mut ch = pin_channel_ref.lock().await;
                            if ch.set_tx_pin(pin_num).is_err() {
                                dbg!("Failed to update TX pin");
                            } else {
                                dbg!("TX pin updated");
                            }
                        } else {
                            dbg!("Invalid TX pin value");
                        }
                    }
                    "UART_RX_PIN" => {
                        let val = a.value()?;
                        dbg!("Updating UART RX pin to ", val);
                        if let Ok(pin_num) = val.parse::<u8>() {
                            let mut ch = pin_channel_ref.lock().await;
                            if ch.set_rx_pin(pin_num).is_err() {
                                dbg!("Failed to update RX pin");
                            } else {
                                dbg!("RX pin updated");
                            }
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

pub(crate) async fn handle_ssh_client(
    stream: &mut TcpSocket<'_>,
    uart: &BufferedUart,
    pin_channel_ref: &'static SunsetMutex<PinChannel>,
) -> Result<(), sunset::Error> {
    // Spawn network tasks to handle incoming connections with demo_common::session()
    let mut inbuf = [0u8; 4096];
    let mut outbuf = [0u8; 4096];

    let ssh_server = SSHServer::new(&mut inbuf, &mut outbuf);
    let (mut rsock, mut wsock) = stream.split();

    let chan_pipe = Channel::<NoopRawMutex, SessionType, 1>::new();

    println!("Calling connection_loop from handle_ssh_client");
    let conn_loop = connection_loop(&ssh_server, &chan_pipe, pin_channel_ref);
    println!("Running server from handle_ssh_client()");
    let server = ssh_server.run(&mut rsock, &mut wsock);

    println!("Setting up serial bridge");

    // TODO: Maybe loop forever here and/or handle disconnection/terminations gracefully?
    let bridge = async {
        let session_type = chan_pipe.receive().await;

        match session_type {
            SessionType::Bridge(ch) => {
                let stdio = ssh_server.stdio(ch).await?;
                let stdio2 = stdio.clone();
                serial_bridge(stdio, stdio2, uart).await?
            }
            // SessionType::Sftp(_ch) => {
            //     // Handle SFTP session
            //     todo!()
            // }
        };
        Ok(())
    };

    println!("Main select() in handle_ssh_client()");
    match select3(conn_loop, server, bridge).await {
        Either3::First(r) => r,
        Either3::Second(r) => r,
        Either3::Third(r) => r,
    }?;

    Ok(())
}
