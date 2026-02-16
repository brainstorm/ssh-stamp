use portable_atomic::{AtomicUsize, Ordering};

use crate::config::SSHStampConfig; //, espressif::buffered_uart::BufferedUart};
use embassy_futures::select::select;
use embassy_sync::pipe::TryWriteError;
/// Wrapper around bidirectional embassy-sync Pipes, in order to handle UART
/// RX/RX happening in an InterruptExecutor at higher priority.
///
/// Doesn't implement the InterruptExecutor, in the task in the app should await
/// the 'run' async function.
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe};
use esp_hal::Async;
use esp_hal::system::software_reset;
use esp_hal::{
    gpio::Pin,
    peripherals::UART1,
    uart::{RxConfig, Uart},
};
use esp_println::dbg;
use static_cell::StaticCell;
use sunset_async::SunsetMutex;

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
    dropped_rx_bytes: AtomicUsize,
}

pub struct Config {}

impl BufferedUart {
    pub fn new() -> Self {
        BufferedUart {
            outward: Pipe::new(),
            inward: Pipe::new(),
            dropped_rx_bytes: AtomicUsize::from(0),
        }
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

                    let mut rx_slice = &uart_rx_buf[..n];

                    // Write rx_slice to 'inward' pipe, dropping bytes rather than blocking if
                    // the pipe is full
                    while !rx_slice.is_empty() {
                        rx_slice = match self.inward.try_write(rx_slice) {
                            Ok(w) => &rx_slice[w..],
                            Err(TryWriteError::Full) => {
                                // If the receive buffer is full (no SSH client, or network congestion) then
                                // drop the oldest bytes from the pipe so we can still write the newest ones.
                                let mut drop_buf = [0u8; UART_BUF_SZ];
                                let dropped = self
                                    .inward
                                    .try_read(&mut drop_buf[..rx_slice.len()])
                                    .unwrap_or_default();
                                let _ = self.dropped_rx_bytes.fetch_update(
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                    |d| Some(d.saturating_add(dropped)),
                                );
                                rx_slice
                            }
                        };
                    }
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

    /// Return the number of dropped bytes (if any) since the last check,
    /// and reset the internal count to 0.
    pub fn check_dropped_bytes(&self) -> usize {
        self.dropped_rx_bytes.swap(0, Ordering::Relaxed)
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

cfg_if::cfg_if! {
   if #[cfg(feature = "esp32")] {
        use esp_hal::peripherals::{GPIO1, GPIO3};
   } else {
       use esp_hal::peripherals::{GPIO10,  GPIO11};
   }
}

cfg_if::cfg_if!(
    if #[cfg(feature = "esp32")] {
        pub struct GPIOS<'a> {
            pub gpio1: GPIO1<'a>,
            pub gpio3: GPIO3<'a>,
        }
    } else {
        pub struct GPIOS<'a> {
            pub gpio10: GPIO10<'a>,
            pub gpio11: GPIO11<'a>,
        }
    }
);

pub const UART_BUFFER_SIZE: usize = 4096;
static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();

pub async fn uart_buffer_wait_for_initialisation() -> &'static BufferedUart {
    UART_BUF.init_with(BufferedUart::new)
}

pub async fn uart_buffer_disable() -> () {
    // disable uart buffer
    software_reset();
}

pub async fn uart_task<'a>(
    uart_buf: &'a BufferedUart,
    uart1: UART1<'a>,
    config: &'a SunsetMutex<SSHStampConfig>,
    gpios: GPIOS<'_>,
) -> Result<(), sunset::Error> {
    dbg!("Configuring UART");
    let config_lock = config.lock().await;
    let rx: u8 = config_lock.uart_pins.rx;
    let tx: u8 = config_lock.uart_pins.tx;
    if rx != tx {
        cfg_if::cfg_if!(
            if #[cfg(feature = "esp32")] {
                let mut holder0 = Some(gpios.gpio1);
                let mut holder1 = Some(gpios.gpio3);
            } else {
                let mut holder0 = Some(gpios.gpio10);
                let mut holder1 = Some(gpios.gpio11);
            }
        );

        let rx_pin = match rx {
            1 => holder0.take().unwrap().degrade(),
            3 => holder1.take().unwrap().degrade(),
            10 => holder0.take().unwrap().degrade(),
            11 => holder1.take().unwrap().degrade(),
            _ => holder0.take().unwrap().degrade(),
        };
        let tx_pin = match tx {
            1 => holder0.take().unwrap().degrade(),
            3 => holder1.take().unwrap().degrade(),
            10 => holder0.take().unwrap().degrade(),
            11 => holder1.take().unwrap().degrade(),
            _ => holder1.take().unwrap().degrade(),
        };

        // Hardware UART setup
        let uart_config = esp_hal::uart::Config::default().with_rx(
            RxConfig::default()
                .with_fifo_full_threshold(16)
                .with_timeout(1),
        );

        let uart = Uart::new(uart1, uart_config)
            .unwrap()
            .with_rx(rx_pin)
            .with_tx(tx_pin)
            .into_async();
        // Run the main buffered TX/RX loop
        uart_buf.run(uart).await;
    }
    // TODO: Pin config error
    Ok(())
}

pub async fn uart_disable() -> () {
    // disable uart
    software_reset();
}
