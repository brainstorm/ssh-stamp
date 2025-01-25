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
use embassy_time::{Duration, Timer};
use esp_hal::rtc_cntl::sleep;
use esp_hal::uart::Uart;
use esp_hal::{peripherals, time, Async};
use esp_hal::peripherals::Peripherals;

// ESP specific
use crate::esp_rng::esp_random;
use esp_println::{dbg, println};
use esp_hal::rng::Trng;
use crate::esp_serial::uart_up;

// Crypto and SSH

pub(crate) async fn handle_ssh_client<'a>(stream: TcpSocket<'a>, uart: Uart<'static, Async>) -> Result<(), EspSshError> {
    // SAFETY: No further (nor concurrent) peripheral operations are happening
    // This will be removed once Trng is cloneable: https://github.com/esp-rs/esp-hal/issues/2372
    let mut peripherals: Peripherals = unsafe {
        peripherals::Peripherals::steal()
    };

    println!("Peripherals stolen at handle_ssh_client()...");
}

pub async fn start(spawner: Spawner) -> Result<(), EspSshError> {
    // Bring up the network interface and start accepting SSH connections.
    let tcp_stack = if_up(spawner).await?;

    // Connect to the serial port
    // TODO: Revisit Result/error.rs wrapping here...
    // TODO: Detection and/or resonable defaults for UART settings... or:
    //       - Make it configurable via settings.rs for now, but ideally...
    //       - ... do what https://keypub.sh does via alternative commands
    //
    let uart = uart_up().await?; 

    accept_requests(tcp_stack, uart).await?;
    // All is fine :)
    Ok(())
}
