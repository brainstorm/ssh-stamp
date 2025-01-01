use core::writeln;
use core::result::Result;
use core::option::Option::{ self, Some, None };

use crate::errors::EspSshError;

use crate::esp_net::{accept_requests, if_up};
use crate::io::AsyncTcpStream;
use crate::keys::{HOST_SECRET_KEY, get_user_public_key};

// Embassy
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use esp_hal::uart::Uart;
use esp_hal::{peripherals, Async};
use esp_hal::peripherals::Peripherals;

// ESP specific
use crate::esp_rng::esp_random;
use esp_println::println;
use esp_hal::rng::Trng;
use crate::esp_serial::uart_up;

// Crypto and SSH
use ed25519_dalek::{SigningKey, VerifyingKey};
use zssh::{AuthMethod, Behavior, PublicKey, Request, SecretKey, Transport};

pub(crate) struct SshServer<'a> {
    stream: AsyncTcpStream<'a>,
    random: Trng<'a>,
    host_secret_key: SecretKey,
    user_public_key: PublicKey,
}

#[derive(Debug, Clone)]
pub(crate) enum ExampleCommand {
    Echo,
    Invalid,
}

impl<'a> Behavior for SshServer<'a> {
    type Stream = AsyncTcpStream<'a>;
    type Command = ExampleCommand;
    type User = &'static str;
    type Random = Trng<'a>;

    fn stream(&mut self) -> &mut Self::Stream {
        &mut self.stream
    }

    fn random(&mut self) -> &mut Self::Random {
        &mut self.random
    }

    fn host_secret_key(&self) -> &SecretKey {
        &self.host_secret_key
    }

    fn allow_user(&mut self, username: &str, auth_method: &AuthMethod) -> Option<&'static str> {
        match (username, auth_method) {
            ("zssh", AuthMethod::PublicKey(public_key)) if *public_key == self.user_public_key => {
                Some("zssh")
            }
            ("guest", AuthMethod::None) => Some("guest"),
            _ => None,
        }
    }

    fn parse_command(&mut self, command: &str) -> Self::Command {
        match command {
            "echo" => ExampleCommand::Echo,
            _ => ExampleCommand::Invalid,
        }
    }
    
    fn server_id(&self) -> &'static str {
        crate::settings::SERVER_ID
    }
    
    fn allow_shell(&self) -> bool {
        true
    }
}

pub(crate) async fn handle_ssh_client<'a>(stream: TcpSocket<'a>, uart: Uart<'static, Async>) -> Result<(), EspSshError> {
    // SAFETY: No further (nor concurrent) peripheral operations are happening
    // This will be removed once Trng is cloneable: https://github.com/esp-rs/esp-hal/issues/2372
    let mut peripherals: Peripherals = unsafe {
        peripherals::Peripherals::steal()
    };

    println!("Peripherals stolen at handle_ssh_client()...");

    let behavior = SshServer {
        stream: AsyncTcpStream(stream),
        random: esp_random(&mut peripherals),
        host_secret_key: SecretKey::Ed25519 {
            secret_key: SigningKey::from_bytes(&HOST_SECRET_KEY),
        },
        user_public_key: PublicKey::Ed25519 {
            public_key: VerifyingKey::from_bytes(&get_user_public_key().0)?,
        },
    };

    let mut packet_buffer = [0u8; 4096]; // the borrowed byte buffer
    let mut transport = Transport::new(&mut packet_buffer, behavior);
    let (mut uart_tx, mut uart_rx) = uart.split();

    loop {
        let channel = transport.accept().await;
        let mut channel = match channel {
            Err(e) => {
                println!("Error accepting request: {:?}", e);
                return Ok(()); // TODO: Handy for quick iteration, not ideal for production.
                               // in any case, it shouldn't panic when client disconnects and 
                               // there's a unexpected EOF.
            }
            Ok(channel) => channel,
        };

        println!(
            "Request {:?} by user {:?} from client {:?}",
            channel.request(), 
            channel.user(),
            channel.client_ssh_id_string()
        );

        match channel.request() {
            Request::Exec(ExampleCommand::Echo) => {
                // This shows how you need to buffer yourself if you need to interleave
                // reads and writes to the channel because the packet buffer is shared.

                let mut buffer = [0u8; 4096];

                loop {
                    let read_len = channel.read_exact_stdin(&mut buffer).await?;

                    if read_len == 0 {
                        break;
                    }

                    channel.write_all_stdout(&buffer[..read_len]).await?;
                }

                channel.exit(0).await?;
            }

            Request::Exec(ExampleCommand::Invalid) => {
                channel
                    .write_all_stderr(b"Sorry, your command was not recognized!\n")
                    .await?;
                channel.exit(1).await?;
            }

            Request::Shell => {
                let mut uart_tx_buffer = [0u8; 4096];
                let mut uart_rx_buffer = [0u8; 4096];

                uart_tx.read_async(&mut uart_tx_buffer).await.unwrap();
                uart_rx.write_async(&mut uart_rx_buffer).await.unwrap();

                channel.write_all_stdout(&uart_tx_buffer).await?;
                uart_rx.flush_async().await.unwrap();
            }
        }
    }
}

pub async fn start(spawner: Spawner) -> Result<(), EspSshError> {
    // Connect to the serial port
    // TODO: Revisit Result/error.rs wrapping here...
    // TODO: Detection and/or resonable defaults for UART settings... or:
    //       - Make it configurable via settings.rs for now, but ideally...
    //       - ... do what https://keypub.sh does via alternative commands
    //
    let uart = uart_up().await?; 

    // Bring up the network interface and start accepting SSH connections.
    let tcp_stack= if_up(spawner).await?;
    accept_requests(tcp_stack, uart).await?;

    // All is fine :)
    Ok(())
}
