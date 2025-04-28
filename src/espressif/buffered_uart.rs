/// Wrapper around bidirectional embassy-sync Pipes, in order to handle UART
/// RX/RX happening in an InterruptExecutor at higher priority.
///
/// Doesn't implement the InterruptExecutor, in the task in the app should await
/// the 'run' async function.
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use embassy_futures::select::select;
use esp_hal::Async;
use esp_hal::uart::Uart;

// Sizes of the software buffers. Inward is more
// important as an overrun here drops bytes. A full outward
// buffer will only block the executor.
const INWARD_BUF_SZ: usize = 512;
const OUTWARD_BUF_SZ: usize = 256;

// Size of the buffer for hardware read/write ops.
const UART_BUF_SZ: usize = 64;

/// Bidirectional pipe buffer for UART communications
pub struct BufferedUart {
    outward: Pipe<CriticalSectionRawMutex, OUTWARD_BUF_SZ>,
    inward: Pipe<CriticalSectionRawMutex, INWARD_BUF_SZ>,
}

pub struct Config {

}

impl BufferedUart {
    pub fn new() -> Self {
        BufferedUart { outward: Pipe::new(), inward: Pipe::new() }
    }

    /// Transfer data between the UART and the buffer struct.
    ///
    /// This should be awaited from an Embassy task that's run
    /// in an InterruptExecutor for lower latency.
    pub async fn run(&self, uart: Uart<'_, Async>) {
        let (mut uart_rx, mut uart_tx) = uart.split();
        let mut uart_rx_buf = [0u8; UART_BUF_SZ];
        let mut uart_tx_buf = [0u8; UART_BUF_SZ];

        loop {
            let rd_from = async {
                loop {
                    let n = uart_rx.read_async(&mut uart_rx_buf).await.unwrap();
                    self.inward.write_all(&uart_rx_buf[..n]).await;
                }
            };
            let rd_to = async {
                loop {
                    let n = self.outward.read(&mut uart_tx_buf).await;
                    // TODO: handle write errors
                    let _ = uart_tx.write_async(&uart_tx_buf[..n]).await;
                }
            };
            select(rd_from, rd_to).await;
        }
    }

    pub async fn read(&self, buf: &mut [u8]) -> usize {
        self.inward.read(buf).await
    }

    pub async fn write(&self, buf: &[u8]) {
        self.outward.write_all(buf).await;
    }

    pub fn reconfigure(&self, _config: Config) {
        todo!();
    }

}

impl Default for BufferedUart {
    fn default() -> Self {
        Self::new()
    }
}
