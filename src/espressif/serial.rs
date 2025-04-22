use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use embassy_futures::select::select;
use esp_hal::Async;
use esp_hal::uart::{ UartRx, UartTx };

const RD_BUF_SZ: usize = 512;
const WR_BUF_SZ: usize = 256;

pub struct BufferedUart<'a> {
    uart_rx: UartRx<'a, Async>,
    uart_tx: UartTx<'a, Async>,
    from_uart: Pipe<CriticalSectionRawMutex, RD_BUF_SZ>,
    to_uart: Pipe<CriticalSectionRawMutex, WR_BUF_SZ>,
}

impl<'a> BufferedUart<'a> {
    pub fn new(self) -> Self {
        return BufferedUart { uart_rx: self.uart_rx, uart_tx: self.uart_tx, from_uart: Pipe::new(), to_uart: Pipe::new() }
    }

    // Call this from inside the embassy Task
    pub async fn run(&self) {
        let mut uart_rx_buf = [0u8; 128];
        let mut uart_tx_buf = [0u8; 128];
        loop {
            let rd_from = async { 
                loop {
                    let n = self.uart_rx.read_async(&mut uart_rx_buf).await.unwrap();
                    self.from_uart.write_all(&uart_rx_buf[..n]).await;
                }
            };
            let rd_to = async {
                loop {
                   let n = self.to_uart.read(&mut uart_tx_buf).await;
                   self.uart_tx.write_async(&uart_tx_buf[..n]).await;
                }
            };
            
            select(rd_from, rd_to).await;
        }
    }
    
    pub async fn read(&self, &mut buf: [u8]) -> usize {
        self.from_uart.read(buf).await
    }

    pub async fn write(&self, &buf: [u8]) {
        self.to_uart.write_all(buf).await
    }

    pub fn reconfigure(&self, config: Config) {
        todo!();
    }

}

