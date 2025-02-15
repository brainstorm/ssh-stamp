use core::writeln;
use core::result::Result;
use core::option::Option::{ self, Some, None };

use crate::esp_net::{accept_requests, if_up};
use crate::esp_rng;
use crate::keys::{self, get_user_public_key};

// Embassy
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::channel::Channel;
use embassy_futures::select::{select, Either};

use esp_hal::clock::CpuClock;
use esp_hal::rng::Rng;
use esp_hal::timer::systimer::Target;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::Uart;
use esp_hal::Async;
use heapless::String;
use sunset::{error, ChanHandle, ServEvent, SignKey};
use sunset_embassy::{ProgressHolder, SSHServer};

use esp_println::{dbg, println};

async fn connection_loop(serv: &SSHServer<'_>, _uart: Uart<'static, Async>) -> Result<(), sunset::Error> {
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
                    let signkey = SignKey::from_openssh(keys::HOST_SECRET_KEY)?;
                    h.hostkeys(&[&signkey])?;
                }
                ServEvent::PasswordAuth(a) => {
                    a.allow();
                }
                | ServEvent::PubkeyAuth(a) => {
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
    let conn_loop = connection_loop(&ssh_server, uart);

    // TODO: This needs a select() which awaits both run() and connection_loop()
    let server = ssh_server.run(&mut rsock, &mut wsock);

    match select(conn_loop, server).await {
        Either::First(r) => r,
        Either::Second(r) => r,
    }?;

    Ok(())
}

pub async fn start(spawner: Spawner) -> Result<(), sunset::Error> {
    // System init
    let peripherals = esp_hal::init({
        let mut config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
        });
    let rng = Rng::new(peripherals.RNG);
    let timg0 = TimerGroup::new(peripherals.TIMG0);

    esp_rng::register_custom_rng(rng);

    cfg_if::cfg_if! {
       if #[cfg(feature = "esp32")] {
            let timg1 = TimerGroup::new(peripherals.TIMG1);
            esp_hal_embassy::init(timg1.timer0);
       } else {
           use esp_hal::timer::systimer::SystemTimer;
           let systimer = SystemTimer::new(peripherals.SYSTIMER).split::<Target>();
           esp_hal_embassy::init(systimer.alarm0);
       }
    }

    let wifi_controller = esp_wifi::init(
            timg0.timer0,
            rng,
            peripherals.RADIO_CLK,
        ).unwrap();

    // Bring up the network interface and start accepting SSH connections.
    let tcp_stack = if_up(spawner, wifi_controller, peripherals.WIFI).await?;

    // Connect to the serial port
    // TODO: Detection and/or resonable defaults for UART settings... or:
    //       - Make it configurable via settings.rs for now, but ideally...
    //       - ... do what https://keypub.sh does via alternative commands
    //
    let (tx_pin, rx_pin) = (peripherals.GPIO10, peripherals.GPIO11);
    let uart = Uart::new(peripherals.UART1, rx_pin, tx_pin)
        .unwrap()
        .into_async();

    accept_requests(tcp_stack, uart).await?;

    // All is fine :)
    Ok(())
}
