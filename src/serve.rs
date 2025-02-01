use core::writeln;
use core::result::Result;
use core::option::Option::{ self, Some, None };

use crate::esp_net::{accept_requests, if_up};
use crate::io::AsyncTcpStream;
use crate::keys::{HOST_SECRET_KEY, get_user_public_key};

// Embassy
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_time::{Duration, Timer};
use esp_hal::uart::Uart;
use esp_hal::{peripherals, time, Async};
use esp_hal::peripherals::Peripherals;
use sunset_embassy::SSHServer;

// ESP specific
use crate::esp_rng::esp_random;
use esp_println::{dbg, println};
use esp_hal::rng::Trng;
use crate::esp_serial::uart_up;

async fn connection_loop(ssh_server: SSHServer<'_>, uart: Uart<'static, Async>) {
    let prog_loop = async {
        loop {
            let mut ph = ProgressHolder::new();
            let ev = serv.progress(&mut ph).await?;
            trace!("ev {ev:?}");
            match ev {
                ServEvent::SessionShell(a) => 
                {
                    if let Some(ch) = common.sess.take() {
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
                        warn!("Too long username")
                    }
                    // handle the rest
                    common.handle_event(ev)?;
                }
                _ => common.handle_event(ev)?,
            };
        };
        #[allow(unreachable_code)]
        Ok::<_, Error>(())
    };
}

pub(crate) async fn handle_ssh_client<'a>(stream: &mut TcpSocket<'a>, uart: Uart<'static, Async>) -> Result<(), sunset::Error> {
    // Spawn network tasks to handle incoming connections with demo_common::session()
    let mut inbuf = [0u8; 4096];
    let mut outbuf= [0u8; 4096];

    let ssh_server = SSHServer::new(&mut inbuf, &mut outbuf)?;
    // Unclear docs: "rsock and wsock are the SSH network channel (TCP port 22 or equivalent)." .... Huh ????
    // Ahhh: rsock == (async) reader_socket, wsock == (async) writer_socket.
    //let rsock = tcp_stack.
    let (mut rsock, mut wsock) = stream.split();

    ssh_server.run(&mut rsock, &mut wsock).await
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
