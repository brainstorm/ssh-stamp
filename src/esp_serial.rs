use esp_backtrace as _;
use esp_hal::{
    timer::timg::TimerGroup,
    uart::{Config, Uart, UartRx, UartTx},
    Async,
};

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

    // rx_fifo_full_threshold
    const READ_BUF_SIZE: usize = 63;

    esp_println::println!("UART init!");
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_hal_embassy::init(timg0.timer0);

    let (tx_pin, rx_pin) = (peripherals.GPIO10, peripherals.GPIO11);
    let config = Config::default().rx_fifo_full_threshold(READ_BUF_SIZE as u16);
    let uart0 = Uart::new_with_config(peripherals.UART0, config, rx_pin, tx_pin)
        .unwrap()
        .into_async();

    Ok(uart0)
}