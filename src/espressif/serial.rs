// use esp_hal::uart::{Config, Uart};
// use esp_hal::Async;
// use esp_hal::peripherals::Peripherals;

// pub(crate) fn init_uart(peripherals: UART1) -> Uart<'static, Async> {
//     let config = Config::default().with_rx_timeout(1);

//     Uart::new(peripherals.UART1, config)
//         .unwrap()
//         .with_rx(peripherals.GPIO11)
//         .with_tx(peripherals.GPIO10)
//         .into_async()
// }

use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use embassy_futures::select::select;
use esp_hal::Async;
use esp_hal::uart::Uart;

const RD_BUF_SZ: usize = 512;
const WR_BUF_SZ: usize = 256;

struct BufferedUart<'a> {
    uart: Uart<'a, Async>,
    from_uart: Pipe<CriticalSectionRawMutex, RD_BUF_SZ>,
    to_uart: Pipe<CriticalSectionRawMutex, WR_BUF_SZ>,
}

impl BufferedUart<'static> {
    pub fn uart_init(spawner: Spawner) -> Self {


    }

    // Call this from inside the embassy Task
    pub async fn run(&mut self) {
        let (mut uart_rx, mut uart_tx) = self.uart.split();
        let mut uart_rx_buf = [0u8; 128];
        let mut uart_tx_buf = [0u8; 128];
        loop {
            let rd_from = async { 
                loop {
                    let n = uart_rx.read(&mut uart_rx_buf).await.unwrap();
                    self.from_uart.write_all(&uart_rx_buf[:n]).await.unwrap();
                }
            };
            let rd_to = async {
                loop {
                   self.to_uart.read(&mut uart_tx_buf);
                   uart_tx.write_all()
                }
            }
            match select(rf_from, rd_to).await {
    
            }
        }
    }
    
    pub async fn read(&mut self, &mut buf: [u8]) -> usize {
        self.from_uart.read(buf).await
    }

    pub async fn write(&mut self, &buf: [u8]) {
        self.to_uart.write_all(buf).await
    }

    pub fn reconfigure(&mut self, config: Config) {
        todo!();
    }

}

