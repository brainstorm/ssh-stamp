use embassy_futures::select::select3;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, pipe::Pipe};
use embedded_io_async::{Read, Write};

// Espressif specific crates
use esp_hal::{uart::{RxError::FifoOverflowed, Uart, UartRx}, Async};
use esp_println::println;

#[embassy_executor::task]
async fn uart_task(instance: BufferedUart<'static>) {
  instance.run();
}

/// Forwards an incoming SSH connection to/from the local UART, until
/// the connection drops
pub(crate) async fn serial_bridge(
    chanr: impl Read<Error = sunset::Error>,
    chanw: impl Write<Error = sunset::Error>,
    uart: Uart<'static, Async>,
) -> Result<(), sunset::Error>
{
    println!("Starting serial <--> SSH bridge");

    // Serial
    let (uart_rx, uart_tx) = uart.split();

    // Need to buffer the UART RX bytes to avoid hardware FIFO overflowing
    // (This is where data can "balloon" out temporarily while waiting for SSH/TCP/Wi-Fi)
    let mut uart_rx_pipe = Pipe::<NoopRawMutex, 512>::new();
    let (rx_pipe_r, rx_pipe_w) = uart_rx_pipe.split();

    select3(uart_read(uart_rx, rx_pipe_w),
            uart_to_ssh(rx_pipe_r, chanw),
            ssh_to_uart(chanr, uart_tx)).await;
    println!("Stopping serial <--> SSH bridge");
    Ok(())
}

async fn uart_to_ssh(mut rx_pipe_r: impl Read, mut chanw: impl Write<Error = sunset::Error>) -> Result<(), sunset::Error> {
    let mut ssh_tx_buf = [0u8; 512];
    loop {
        let n = rx_pipe_r.read(&mut ssh_tx_buf).await.unwrap();
        chanw.write_all(&ssh_tx_buf[..n]).await?;
    }
}

/// Read from the UART, write to the pipe end
async fn uart_read(mut uart_rx: UartRx<'_, Async>, mut rx_pipe_w: impl Write<>) -> Result<(), sunset::Error> {
    let mut uart_rx_buf = [0u8; 128];
    loop {
        match uart_rx.read_async(&mut uart_rx_buf).await {
            Ok(n) => {
                rx_pipe_w.write_all(&uart_rx_buf[..n]).await.unwrap(); // TODO: handle error
            },
            Err(e) => match e {
                FifoOverflowed => {
                    // this will happen if the SSH link gets slowed down or
                    // is about to time out
                    println!("UART RX FIFO overflowed, bytes were lost");
                },
                _ => todo!(), // TODO: Need to handle (or ignore) other intermittent UART errors
            },
        }
    }
}

// Unlike reading from the UART, SSH comes with flow control
// so no need to add an intermediate buffer layer
async fn ssh_to_uart(mut chanr: impl Read<Error = sunset::Error>, mut uart_tx: esp_hal::uart::UartTx<'_, Async>) -> Result<(), sunset::Error> {
    let mut uart_tx_buf = [0u8; 64];
    loop {
        let n = chanr.read(&mut uart_tx_buf).await?;
        if n == 0 {
            return Err(sunset::Error::ChannelEOF);
        }
        let uart_tx_buf = &mut uart_tx_buf[..n];
        uart_tx.write_async(uart_tx_buf).await.unwrap(); // TODO: return error
    }
}
