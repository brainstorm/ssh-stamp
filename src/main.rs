#![no_std]
#![no_main]

use core::{any::Any, iter::Map, marker::Sized};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    gpio::AnyPin, interrupt::{software::SoftwareInterruptControl, Priority}, peripherals::{self, UART1}, rng::Rng, timer::timg::TimerGroup, uart::{Config, RxConfig, Uart}
};
use esp_hal_embassy::InterruptExecutor;

use esp_storage::FlashStorage;
use embassy_executor::Spawner;
use ssh_stamp::{config::SSHConfig, espressif::{
    buffered_uart::BufferedUart,
    net::{accept_requests, if_up},
    rng,
}, storage::Fl};
use static_cell::StaticCell;
use sunset_async::SunsetMutex;
use heapless::Vec;

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    cfg_if::cfg_if!(
        if #[cfg(feature = "esp32s2")] {
            // TODO: This heap size will crash at runtime, we need to fix this
            // applying ideas from https://github.com/brainstorm/ssh-stamp/pull/41#issuecomment-2964775170
                esp_alloc::heap_allocator!(size: 69 * 1024);
        } else {
                esp_alloc::heap_allocator!(size: 72 * 1024);
        }
    );
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
    let config_for_network = config;
    let tcp_stack = if_up(spawner, wifi_controller, peripherals.WIFI, &mut rng, config_for_network)
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
    // Grab UART1, typically not connected to dev board's TTL2USB IC nor builtin JTAG functionality
    let uart1 = peripherals.UART1;

    // Potential pins to use for such UART, to be owned by uart_task.
    // TODO: Unsure if that's what was referred in the conversations below...
    static UART_PINS: StaticCell<Vec<AnyPin<'static>, 2>> = StaticCell::new();
    let uart_pins = UART_PINS.init({
        let mut pins = Vec::<AnyPin<'static>, 2>::new();
        pins.push(peripherals.GPIO1.into()).unwrap();
        pins.push(peripherals.GPIO2.into()).unwrap();
        pins
    });

    // let rx = config.lock().await.uart_rx_pin;
    // Use the same config reference for UART task.
    cfg_if::cfg_if! {
        if #[cfg(not(feature = "esp32c2"))] {
            interrupt_spawner.spawn(uart_task(uart_buf, uart1, uart_pins, config)).unwrap(); //, _config)).unwrap();
        } else {
            interrupt_spawner.spawn(uart_task(uart_buf, uart1, uart_pins, config)).unwrap(); //, config)).unwrap();
        }
    }
    accept_requests(tcp_stack, uart_buf).await;
}

static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();
static INT_EXECUTOR: StaticCell<InterruptExecutor<0>> = StaticCell::new();

#[embassy_executor::task()]
async fn uart_task(
    buffer: &'static BufferedUart,
    uart_periph: UART1<'static>,
    uart_pins: &'static Vec<AnyPin<'static>, 2>,
    config: &'static SunsetMutex<SSHConfig>
) {
    // Suggestions from esp-rs/esp-hal matrix channel by different authors on how to handle runtime UART pin changes, WIP:

    // You can do all of this without steal by passing the pins and uart with .reborrow() appended (so that we borrow the pins/uart, not move), 
    // then if you drop the uart driver all the resources will still be there so you can construct again
    // If you just need to set the config again, just call apply_config

    // are you reading this from a non-rust file or something? I'd expect a cross-platform crate to take pins as some generic parameter and use a 
    // non-chip-specific trait to access them, not use integers. Should be straightforward enough at compile-time and if you're doing it at runtime 
    // that seems a bit weird.

    // I think to do that, you could have the uart task own all of the AnyPins that might be chosen for uart on that platform, as well as uart_periph = UART1. 
    // Turn the pins array into an array of PeripheralRef<AnyPin> with .into_ref(), and also uart_periph.into_ref(). then when you want to configure, 
    // like mabez said you .reborrow() from chosen pins in that array. (might need an array of refcells?). then pass the reborrowed uart_tx/uart_rx into the 
    // .with_rx or .with_tx? Then when you next reconfigure, you drop the "reborrow" instances and then the borrow checker should let you borrow again.

    // yeah, and if you want it to put them to different uses then you can probably build a map to hold the pins you aren't currently using in a mutex or the like, 
    // so you can check that the configuration is sensible at runtime.

    // TODO: Probably better use a HashMap as suggested above instead of an array of pins?
    let rx_pin_num = &uart_pins[config.lock().await.uart_rx_pin as usize];
    let tx_pin_num = &uart_pins[config.lock().await.uart_tx_pin as usize];

    // Hardware UART setup
    let uart_config = Config::default().with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1)
    );

    let uart = Uart::new(uart_periph, uart_config)
        .unwrap()
        .with_rx(rx_pin_num)
        .with_tx(tx_pin_num)
        .into_async();

    // Run the main buffered TX/RX loop
    buffer.run(uart).await;
}
