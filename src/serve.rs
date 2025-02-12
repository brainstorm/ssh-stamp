use core::writeln;
use core::result::Result;
use core::option::Option::{ self, Some, None };

use crate::esp_net::{accept_requests, if_up};
use crate::keys::{self};

// Embassy
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::channel::Channel;
use esp_hal::uart::Uart;
use esp_hal::Async;
use heapless::String;
use sunset::{error, ChanHandle, Error, ServEvent, SignKey};
use sunset_embassy::{ProgressHolder, SSHServer};

use esp_println::{dbg, println};
use crate::esp_serial::uart_up;

async fn connection_loop(serv: SSHServer<'_>, _uart: Uart<'static, Async>) -> Result<(), sunset::Error> {
    let username = Mutex::<NoopRawMutex, _>::new(String::<20>::new());
    let chan_pipe = Channel::<NoopRawMutex, ChanHandle, 1>::new();
    let mut session: Option::<ChanHandle> = None;
    
    println!("Entering connection_loop and prog_loop is next...");

    loop {
            let mut ph = ProgressHolder::new();
            let ev = serv.progress(&mut ph).await?;
            dbg!(&ev);
            match ev {
                ServEvent::SessionShell(a) => 
                {
                    if let Some(ch) = session.take() {
                        debug_assert!(ch.num() == a.channel()?);
                        a.succeed()?;
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
                    // FIXME: Is this the right key to pass here? Perhaps get_user_public_key() should be here?
                    let signkey = SignKey::from_openssh(keys::HOST_SECRET_KEY)?;
                    h.hostkeys(&[&signkey])?;
                }
                ServEvent::PasswordAuth(_a) => {
                   // TODO: disallow password auth
                }
                | ServEvent::PubkeyAuth(_a) => {
                    // TODO: handle!
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
                ServEvent::SessionPty(a) => {
                    a.succeed()?;
                }
                ServEvent::SessionExec(a) => {
                    a.fail()?;
                }
                | ServEvent::Defunct
                | ServEvent::SessionShell(_) => {
                    println!("Expected caller to handle event");
                    //error!("Expected caller to handle {event:?}");
                    error::BadUsage.fail()?
                }
            };
        };
}


pub(crate) async fn handle_ssh_client<'a>(stream: &mut TcpSocket<'a>, uart: Uart<'static, Async>) -> Result<(), sunset::Error> {
    // Spawn network tasks to handle incoming connections with demo_common::session()
    let mut inbuf = [0u8; 4096];
    let mut outbuf= [0u8; 4096];

    let ssh_server = SSHServer::new(&mut inbuf, &mut outbuf)?;
    let (mut rsock, mut wsock) = stream.split();

    println!("Calling connection_loop from handle_ssh_client");
    // FIXME: This should be a spawned, never-ending task.
    connection_loop(ssh_server, uart).await?;

    // TODO: This needs a select() which awaits both run() and connection_loop()
    //ssh_server.run(&mut rsock, &mut wsock).await
    Ok(())
}

pub async fn start(spawner: Spawner) -> Result<(), sunset::Error> {
    // Bring up the network interface and start accepting SSH connections.
    let tcp_stack = if_up(spawner).await?;

    // Connect to the serial port
    // TODO: Detection and/or resonable defaults for UART settings... or:
    //       - Make it configurable via settings.rs for now, but ideally...
    //       - ... do what https://keypub.sh does via alternative commands
    //
    let uart = uart_up().await?; 

    accept_requests(tcp_stack, uart).await?;

    // All is fine :)
    Ok(())
}
