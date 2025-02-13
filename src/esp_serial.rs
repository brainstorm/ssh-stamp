use esp_backtrace as _;
use esp_hal::{
    uart::{UartRx, UartTx}, Async
};

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
