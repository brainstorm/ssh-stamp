// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The hardware tests for the UART code which runs against a UART peripheral with its
//! TX looped back into the RX through the GPIOs.

#![no_std]
#![no_main]

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;

// Links the HIL library crate for the bootloader and getrandom initialization.
use ssh_stamp_esp32_hil as _;

/// Mock the channel read required on the [`serial_bridge`], this will move the bytes
/// through the pipe.
struct ChanRead<'a>(&'a Pipe<CriticalSectionRawMutex, 512>);

impl embedded_io_async::ErrorType for ChanRead<'_> {
    type Error = sunset::Error;
}

impl embedded_io_async::Read for ChanRead<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(self.0.read(buf).await)
    }
}

/// Mock the channel write required on the [`serial_bridge`], this will move the bytes
/// through the pipe.
struct ChanWrite<'a>(&'a Pipe<CriticalSectionRawMutex, 512>);

impl embedded_io_async::ErrorType for ChanWrite<'_> {
    type Error = sunset::Error;
}

impl embedded_io_async::Write for ChanWrite<'_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        Ok(self.0.write(buf).await)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[embedded_test::tests(default_timeout = 30, executor = esp_rtos::embassy::Executor::new())]
mod tests {
    use super::{ChanRead, ChanWrite};
    use core::array::from_fn;
    use embassy_futures::select::{Either, select};
    use embassy_sync::pipe::Pipe;
    use ssh_stamp::serial::serial_bridge;
    use ssh_stamp_esp32::{
        BufferedUart, EspUartPins, UART_SIGNAL, boot, spawn_uart, start_interrupt_executor,
    };
    use ssh_stamp_esp32_boards::take_uart_pins;
    use ssh_stamp_hal::UartParams;

    pub struct Context {
        uart_buf: &'static BufferedUart,
    }

    async fn read_bytes(mut read: impl AsyncFnMut(&mut [u8]) -> usize, buf: &mut [u8]) {
        let mut filled = 0;
        while filled < buf.len() {
            filled += read(&mut buf[filled..]).await;
        }
    }

    #[init]
    fn init() -> Context {
        boot!(peripherals, _rng, _entropy_source, sw_int1);

        let (rx_pin, tx_pin, _rx_num, _tx_num) = take_uart_pins!(peripherals);
        let pins = EspUartPins {
            rx: rx_pin,
            tx: tx_pin,
        };

        let spawner = start_interrupt_executor(sw_int1);
        let uart_buf = spawn_uart(spawner, peripherals.UART1, pins, UartParams::default());
        UART_SIGNAL.signal(0);

        Context { uart_buf }
    }

    #[test]
    async fn write_bytes_through_uart(context: Context) {
        let uart_buf = context.uart_buf;

        let sent: [u8; 256] = from_fn(|i| u8::try_from(i).unwrap());
        uart_buf.write(&sent).await;

        let mut received = [0u8; 256];
        read_bytes(async |buf| uart_buf.read(buf).await, &mut received).await;

        assert_eq!(received, sent);
        assert_eq!(uart_buf.check_dropped_bytes(), 0);
    }

    #[test]
    async fn serial_bridge_ssh_bytes(context: Context) {
        let uart_buf = context.uart_buf;

        let to_uart: Pipe<_, 512> = Pipe::new();
        let from_uart: Pipe<_, 512> = Pipe::new();
        let bridge = serial_bridge(ChanRead(&to_uart), ChanWrite(&from_uart), uart_buf);

        let receive_function = async {
            to_uart.write_all(b"test").await;

            let mut received = [0u8; 4];
            read_bytes(async |buf| from_uart.read(buf).await, &mut received).await;
            received
        };

        // Run the bridge until the receive function finishes.
        match select(bridge, receive_function).await {
            Either::First(result) => panic!("the bridge stopped on its own: {result:?}"),
            Either::Second(received) => assert_eq!(received.as_slice(), b"test"),
        }
    }
}
