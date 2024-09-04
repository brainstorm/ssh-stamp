use core::writeln;
use core::result::Result;
use core::option::Option::{ self, Some, None };
use core::unreachable;

use crate::esp_net::ifup;
use crate::io::{AsyncTcpStream, DebuggableTcpSocket};

use embassy_executor::Spawner;
// Embassy
use embassy_net::tcp::Error;

// ESP specific
use crate::esp_rng::esp_random;
use esp_println::println;
use esp_hal::peripherals::Peripherals;
use esp_hal::rng::Trng;

use ed25519_dalek::{SigningKey, VerifyingKey};
use zssh::{AuthMethod, Behavior, PublicKey, Request, SecretKey, Transport, TransportError};

struct SshServer<'a> {
    stream: AsyncTcpStream<'a>,
    random: Trng<'a>,
    host_secret_key: SecretKey,
    user_public_key: PublicKey,
}

#[derive(Debug, Clone)]
enum ExampleCommand {
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
        "SSH-2.0-esp-hosted-ssh-0.1"
    }
    
    fn allow_shell(&self) -> bool {
        false
    }
}

// Randomly created host identity.
const HOST_SECRET_KEY: [u8; 32] = [
    0xdf, 0x77, 0xbb, 0xf9, 0xf6, 0x42, 0x04, 0x40, 0x4c, 0x69, 0xe7, 0x1c, 0x7c, 0x6c, 0xda, 0x71,
    0x6c, 0xdc, 0x20, 0xa3, 0xe1, 0x2f, 0x78, 0x4a, 0x6d, 0xaa, 0x96, 0x3a, 0x1a, 0x51, 0xea, 0x4f,
];

// Matches examples/zssh.priv key.
const USER_PUBLIC_KEY: [u8; 32] = [
    0xa5, 0x34, 0xb0, 0xa8, 0x36, 0x95, 0x45, 0x22, 0xd2, 0x75, 0x46, 0xba, 0x6b, 0x17, 0xdc, 0xc9,
    0x18, 0xfb, 0x9d, 0xeb, 0xe2, 0xd5, 0x36, 0x5e, 0x1b, 0xdb, 0xca, 0x32, 0xb5, 0xbd, 0x90, 0xb4,
];

async fn handle_client(stream: DebuggableTcpSocket<'_>) -> Result<(), TransportError<SshServer>> {
    let mut peripherals = Peripherals::take();
    let behavior = SshServer {
        stream: AsyncTcpStream(stream),
        random: esp_random(&mut peripherals),
        host_secret_key: SecretKey::Ed25519 {
            secret_key: SigningKey::from_bytes(&HOST_SECRET_KEY),
        },
        user_public_key: PublicKey::Ed25519 {
            public_key: VerifyingKey::from_bytes(&USER_PUBLIC_KEY).unwrap(),
        },
    };

    let mut packet_buffer = [0u8; 4096]; // the borrowed byte buffer
    let mut transport = Transport::new(&mut packet_buffer, behavior);

    loop {
        let mut channel = transport.accept().await?;

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

            Request::Shell => unreachable!("shell requests not allowed"),
        }
    }
}

pub async fn start(spawner: Spawner) -> Result<(), Error> {
    //let listener = TcpListener::bind("127.0.0.1:2222").await?;

    ifup(spawner).await;
    Ok(())
    // loop {
    //     let (stream, _) = listener.accept().await?;

    //     if let Err(error) = handle_client(stream).await {
    //         println!("Transport error: {:?}", error);
    //     }
    // }
}