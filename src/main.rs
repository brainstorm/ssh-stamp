#![no_std]
#![no_main]

use core::marker::Sized;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    gpio::Pin, interrupt::{software::SoftwareInterruptControl, Priority}, peripherals::UART1, rng::Rng, timer::timg::TimerGroup, uart::{Config, RxConfig, Uart}
};
use esp_hal_embassy::InterruptExecutor;

use esp_println::dbg;
use esp_storage::FlashStorage;
use embassy_executor::Spawner;
use ssh_stamp::{config::SSHConfig, espressif::{
    buffered_uart::BufferedUart,
    net::{accept_requests, if_up},
    rng,
}, storage::Fl};
use static_cell::StaticCell;
use sunset_async::SunsetMutex;
use ssh_stamp::config::GPIOConfig;
use ssh_stamp::config::PinChannel;

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32s2")] {
            // TODO: This heap size will crash at runtime (only for the ESP32S2), we need to fix this
            // applying ideas from https://github.com/brainstorm/ssh-stamp/pull/41#issuecomment-2964775170
                esp_alloc::heap_allocator!(size: 69 * 1024);
        } else {
                esp_alloc::heap_allocator!(size: 72 * 1024);
        }
    );
    esp_bootloader_esp_idf::esp_app_desc!();
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

    // Read SSH configuration from Flash (if it exists)
    let mut flash = Fl::new(FlashStorage::new());
    let config = ssh_stamp::storage::load_or_create(&mut flash).await;

    static FLASH: StaticCell<SunsetMutex<Fl>> = StaticCell::new();
    let _flash = FLASH.init(SunsetMutex::new(flash));

    static CONFIG: StaticCell<SunsetMutex<SSHConfig>> = StaticCell::new();
    let config = CONFIG.init(SunsetMutex::new(config.unwrap()));

    let wifi_controller = esp_wifi::init(timg0.timer0, rng, peripherals.RADIO_CLK).unwrap();

    // Bring up the network interface and start accepting SSH connections.
    // Clone the reference to config to avoid borrow checker issues.
    let tcp_stack = if_up(spawner, wifi_controller, peripherals.WIFI, &mut rng, config)
        .await
        .unwrap();

    // Set up software buffered UART to run in a higher priority InterruptExecutor
    let uart_buf = UART_BUF.init_with(BufferedUart::new);
    let software_interrupts = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let interrupt_executor =
        INT_EXECUTOR.init_with(|| InterruptExecutor::new(software_interrupts.software_interrupt0));
    cfg_if::cfg_if! {
        if #[cfg(any(feature = "esp32", feature = "esp32s2", feature = "esp32s3"))] {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority1);
        } else {
            let interrupt_spawner = interrupt_executor.start(Priority::Priority10);
        }
    }

    let serde_pin_config = {
        let guard = config.lock().await;
        guard.uart_pins.clone()
    };


    let available_gpios = GPIOConfig {
        gpio10: Some(peripherals.GPIO10.degrade()),
        gpio11: Some(peripherals.GPIO11.degrade()),
    };
    
    static CHANNEL: StaticCell<PinChannel> = StaticCell::new();
    let channel = CHANNEL.init({
        PinChannel::new(serde_pin_config, available_gpios)
    });

    // Grab UART1, typically not connected to dev board's TTL2USB IC nor builtin JTAG functionality
    let uart1 = peripherals.UART1;

    // Use the same config reference for UART task.
    interrupt_spawner.spawn(uart_task(uart_buf, uart1, channel)).unwrap();

    accept_requests(tcp_stack, uart_buf).await;
}

static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();
static INT_EXECUTOR: StaticCell<InterruptExecutor<0>> = StaticCell::new();

#[embassy_executor::task()]
async fn uart_task(
    buffer: &'static BufferedUart,
    uart_periph: UART1<'static>,
    channel: &'static mut PinChannel,
) {
    dbg!("spawning UART task...");
    // Hardware UART setup
    let uart_config = Config::default().with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1)
    );

    dbg!("before with_channel");
    // Sync pin config via channels
    channel.with_channel(async |rx, tx| {
        dbg!("into with_channel");
        let uart = Uart::new(uart_periph, uart_config)
            .unwrap()
            .with_rx(rx)
            .with_tx(tx)
            .into_async();

        // Run the main buffered TX/RX loop
        dbg!("before buffer_run");
        buffer.run(uart).await;
        dbg!("after buffer_run");
    }).await.unwrap();
}
