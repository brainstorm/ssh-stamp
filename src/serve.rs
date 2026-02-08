// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::option::Option::{self, None, Some};
use core::result::Result;

use crate::espressif::buffered_uart::BufferedUart;
use crate::keys;
use crate::serial::serial_bridge;

// Embassy
use embassy_futures::select::{select3, Either3};
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;

use heapless::String;
use sunset::{error, ChanHandle, ServEvent, SignKey};
use sunset_async::{ProgressHolder, SSHServer};

use esp_println::{dbg, println};

async fn connection_loop(
    serv: &SSHServer<'_>,
    chan_pipe: &Channel<NoopRawMutex, ChanHandle, 1>,
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
                    a.succeed()?;
                    dbg!("We got shell");
                    let _ = chan_pipe.try_send(ch);
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
            ServEvent::SessionSubsystem(_) => (),
        }
    }
}

pub(crate) async fn handle_ssh_client(
    stream: &mut TcpSocket<'_>,
    uart: &BufferedUart,
) -> Result<(), sunset::Error> {
    // Spawn network tasks to handle incoming connections with demo_common::session()
    let mut inbuf = [0u8; 4096];
    let mut outbuf = [0u8; 4096];

    let ssh_server = SSHServer::new(&mut inbuf, &mut outbuf);
    let (mut rsock, mut wsock) = stream.split();

    let chan_pipe = Channel::<NoopRawMutex, ChanHandle, 1>::new();

    println!("Calling connection_loop from handle_ssh_client");
    let conn_loop = connection_loop(&ssh_server, &chan_pipe);
    println!("Running server from handle_ssh_client()");
    let server = ssh_server.run(&mut rsock, &mut wsock);

    println!("Setting up serial bridge");
    let bridge = async {
        let ch = chan_pipe.receive().await;
        let stdio = ssh_server.stdio(ch).await?;
        let stdio2 = stdio.clone();
        serial_bridge(stdio, stdio2, uart).await
    };

    println!("Main select() in handle_ssh_client()");
    match select3(conn_loop, server, bridge).await {
        Either3::First(r) => r,
        Either3::Second(r) => r,
        Either3::Third(r) => r,
    }?;

    Ok(())
}
