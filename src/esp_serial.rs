use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::{
    timer::timg::TimerGroup,
    uart::{Config, Uart, UartRx, UartTx},
    Async,
};

// rx_fifo_full_threshold
const READ_BUF_SIZE: usize = 64;

#[embassy_executor::task]
async fn writer(mut tx: UartTx<'static, Async>, serial_tx_ring_buf: &'static mut [u8]) {
    //use core::fmt::Write;
    embedded_io_async::Write::write(
        &mut tx,
        serial_tx_ring_buf,
    )
    .await
    .unwrap();

    loop {
        embedded_io_async::Write::flush(&mut tx).await.unwrap();
    }

    // write!(&mut tx, "\r\n-- received {} bytes --\r\n", bytes_read).unwrap();
    // embedded_io_async::Write::flush(&mut tx).await.unwrap();
}

#[embassy_executor::task]
async fn reader(mut rx: UartRx<'static, Async>, serial_rx_ring_buf: &'static mut [u8]) {
    loop {
        let r = embedded_io_async::Read::read(&mut rx, serial_rx_ring_buf).await;
        match r {
            Ok(len) => {
                esp_println::println!("Read: {len}, data: {:?}", serial_rx_ring_buf);
            }
            Err(e) => esp_println::println!("RX Error: {:?}", e),
        }
    }
}

#[embassy_executor::task]
pub(crate) async fn open_uart(spawner: Spawner, serial_rx_ring_buf: &'static mut [u8], 
                                                serial_tx_ring_buf: &'static mut [u8]) {
    esp_println::println!("UART init!");
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_hal_embassy::init(timg0.timer0);

    let (tx_pin, rx_pin) = (peripherals.GPIO16, peripherals.GPIO17);
    let config = Config::default().rx_fifo_full_threshold(READ_BUF_SIZE as u16);
    let uart0 = Uart::new_with_config(peripherals.UART0, config, rx_pin, tx_pin)
        .unwrap()
        .into_async();

    let (rx, tx) = uart0.split();

    spawner.spawn(reader(rx, serial_rx_ring_buf)).ok();
    spawner.spawn(writer(tx, serial_tx_ring_buf)).ok();
}