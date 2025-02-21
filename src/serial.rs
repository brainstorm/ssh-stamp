use embassy_futures::select::select;
use embedded_io_async::{Read, Write};

// Espressif specific crates
use esp_hal::{uart::Uart, Async};
use esp_println::{dbg, println};



/// Forwards an incoming SSH connection to a local serial port, either uart or USB
pub(crate) async fn serial_bridge<R, W>(
    chanr: &mut R,
    chanw: &mut W,
    uart: Uart<'static, Async>,
) -> Result<(), sunset::Error>
where
    R: Read<Error = sunset::Error>,
    W: Write<Error = sunset::Error>,
{
    println!("Starting serial <--> SSH bridge");

    // Serial
    let (mut uart_rx, mut uart_tx) = uart.split();

    let r = async {
        // TODO: could have a single buffer to translate in-place.
        let mut uart_rx_buf = [0u8; 64];
        loop {
            let n = uart_rx.read_async(&mut uart_rx_buf).await.unwrap(); // TODO: return error
            let uart_rx_buf = &mut uart_rx_buf[..n];
            chanw.write_all(uart_rx_buf).await?;
        }
        #[allow(unreachable_code)]
        Ok::<(), sunset::Error>(())
    };
    let w = async {
        let mut uart_tx_buf = [0u8; 64];
        loop {
            let n = chanr.read(&mut uart_tx_buf).await?;
            dbg!(n);
            if n == 0 {
                return Err(sunset::Error::ChannelEOF);
            }
            let uart_tx_buf = &mut uart_tx_buf[..n];
            uart_tx.write_async(uart_tx_buf).await.unwrap();
        }
        #[allow(unreachable_code)]
        Ok::<(), sunset::Error>(())
    };

    select(r, w).await;
    println!("Stopping serial <--> SSH bridge");
    Ok(())
}
