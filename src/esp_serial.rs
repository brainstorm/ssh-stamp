use esp_backtrace as _;
use esp_hal::{
    peripherals::{self, Peripherals}, uart::{Config, Uart, UartRx, UartTx}, Async
};
use esp_println::println;

use crate::errors::EspSshError;

#[embassy_executor::task]
async fn writer(mut tx: UartTx<'static, Async>, serial_tx_ring_buf: &'static mut [u8]) {
    let tx_writer = tx.write_async(serial_tx_ring_buf).await;

    match tx_writer {
        Ok(len) => {
            esp_println::println!("Wrote: {len}, data: {:?}", serial_tx_ring_buf);
        }
        Err(e) => esp_println::println!("TX Error: {:?}", e),
    }
}

#[embassy_executor::task]
async fn reader(mut rx: UartRx<'static, Async>, serial_rx_ring_buf: &'static mut [u8]) {
    loop {
        let rx_reader = rx.read_async(serial_rx_ring_buf).await;
        match rx_reader {
            Ok(len) => {
                esp_println::println!("Read: {len}, data: {:?}", serial_rx_ring_buf);
            }
            Err(e) => esp_println::println!("RX Error: {:?}", e),
        }
    }
}

pub(crate) async fn uart_up() -> Result<Uart<'static, Async>, EspSshError> {
    esp_println::println!("UART init!");

    // SAFETY: No concurrent peripheral operations are happening at this point???
    // FIXME: Concerning since we steal it in handle_ssh_client() as well
    let peripherals: Peripherals = unsafe {
        peripherals::Peripherals::steal()
    };

    println!("Peripherals stolen at uart_up()...");

    let (tx_pin, rx_pin) = (peripherals.GPIO10, peripherals.GPIO11);
    let uart0 = Uart::new(peripherals.UART0, rx_pin, tx_pin)
        .unwrap()
        .into_async();

    Ok(uart0)
}