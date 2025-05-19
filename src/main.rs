#![no_std]
#![no_main]

use core::marker::Sized;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{gpio::AnyPin, interrupt::{software::SoftwareInterruptControl, Priority}, peripherals::UART1, rng::Rng, timer::timg::TimerGroup, uart::{Config, RxConfig, Uart}};
use esp_hal_embassy::InterruptExecutor;

use embassy_executor::Spawner;
use ssh_stamp::espressif::{net::{accept_requests, if_up}, rng, buffered_uart::BufferedUart};
use static_cell::StaticCell;

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    esp_alloc::heap_allocator!(size: 72 * 1024);
    esp_println::logger::init_logger_from_env();

    // System init
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut rng = Rng::new(peripherals.RNG);
    let timg0 = TimerGroup::new(peripherals.TIMG0);

    rng::register_custom_rng(rng);

    cfg_if::cfg_if! {
       if #[cfg(feature = "esp32")] {
            let timg1 = TimerGroup::new(peripherals.TIMG1);
            esp_hal_embassy::init(timg1.timer0);
       } else {
           use esp_hal::timer::systimer::SystemTimer;
           let systimer = SystemTimer::new(peripherals.SYSTIMER);
           esp_hal_embassy::init(systimer.alarm0);
       }
    }

    let wifi_controller = esp_wifi::init(timg0.timer0, rng, peripherals.RADIO_CLK).unwrap();

    // Bring up the network interface and start accepting SSH connections.
    let tcp_stack = if_up(spawner, wifi_controller, peripherals.WIFI, &mut rng).await.unwrap();

    // Set up software buffered UART to run in a higher priority InterruptExecutor
    let uart_buf = UART_BUF.init_with(BufferedUart::new);
    let software_interrupts = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let interrupt_exeuctor = INT_EXECUTOR.init_with(|| InterruptExecutor::new(software_interrupts.software_interrupt0));
    cfg_if::cfg_if! {
        if #[cfg(any(feature = "esp32", feature = "esp32s2", feature = "esp32s3"))] {
            let interrupt_spawner = interrupt_exeuctor.start(Priority::Priority1);
        } else {
            let interrupt_spawner = interrupt_exeuctor.start(Priority::Priority10);
        }
    }
    interrupt_spawner.spawn(uart_task(uart_buf, peripherals.UART1, peripherals.GPIO11.into(), peripherals.GPIO10.into())).unwrap();

    accept_requests(tcp_stack, uart_buf).await;
}

static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();

static INT_EXECUTOR: StaticCell<InterruptExecutor<0>> = StaticCell::new();

#[embassy_executor::task]
async fn uart_task(buffer: &'static BufferedUart, uart_periph: UART1, rx_pin: AnyPin, tx_pin: AnyPin) {
    // Hardware UART setup
    let uart_config = Config::default()
        .with_rx(RxConfig::default().with_fifo_full_threshold(16).with_timeout(1));

    let uart = Uart::new(uart_periph, uart_config)
        .unwrap()
        .with_rx(rx_pin)
        .with_tx(tx_pin)
        .into_async();

    // Run the main buffered TX/RX loop
    buffer.run(uart).await;
}
