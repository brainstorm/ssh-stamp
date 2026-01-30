#![no_std]
#![no_main]

use core::marker::Sized;
use embassy_embedded_hal::flash;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    gpio::Pin,
    interrupt::{software::SoftwareInterruptControl, Priority},
    peripherals::UART1,
    rng::Rng,
    timer::timg::TimerGroup,
    uart::{Config, RxConfig, Uart},
};
//use esp_radio::wifi::event;
use esp_rtos::embassy::InterruptExecutor;

use embassy_executor::Spawner;
use esp_println::dbg;
use esp_storage::FlashStorage;
use ssh_stamp::pins;
use ssh_stamp::pins::GPIOConfig;
use ssh_stamp::pins::PinChannel;
use ssh_stamp::{
    config::SSHStampConfig,
    espressif::{
        buffered_uart::BufferedUart,
        net::{accept_requests, if_up},
        rng,
    },
    storage::Fl,
};
use static_cell::StaticCell;
use sunset_async::SunsetMutex;

#[esp_rtos::main]
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
    let mut rng = Rng::new();
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);

    // Set event handlers for wifi before init to avoid missing any.
    // let mut connections = 0u32;
    // _ = event::ApStart::replace_handler(|_| println!("ap start event"));
    // event::ApStaConnected::update_handler(move |event| {
    //     connections += 1;
    //     esp_println::println!("connected {}, mac: {:?}", connections, event.mac());
    // });
    // event::ApStaConnected::update_handler(|event| {
    //     esp_println::println!("connected aid: {}", event.aid());
    // });
    // event::ApStaDisconnected::update_handler(|event| {
    //     println!(
    //         "disconnected mac: {:?}, reason: {:?}",
    //         event.mac(),
    //         event.reason()
    //     );
    // });

    rng::register_custom_rng(rng);

    cfg_if::cfg_if! {
       if #[cfg(feature = "esp32")] {
            let timg1 = TimerGroup::new(peripherals.TIMG1);
            esp_rtos::start(timg1.timer0, sw_int);
       } else {
           use esp_hal::timer::systimer::SystemTimer;
           let systimer = SystemTimer::new(peripherals.SYSTIMER);
           esp_rtos::start(systimer.alarm0, sw_int);
       }
    }

    // Read SSH configuration from Flash (if it exists)
    let config = {

        // let rrr = flash::get_flash_n_buffer();
        let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
            panic!("Could not acquire flash storage lock");
        };

        let mut flash_storage = flash_storage_guard.lock().await;
        // TODO: Migrate this function/test to embedded-test.
        // Quick roundtrip test for SSHStampConfig
        // ssh_stamp::config::roundtrip_config();

        ssh_stamp::storage::load_or_create(&mut flash_storage).await

    }

    .expect("Could not load or create SSHStampConfig");
    static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();
	
    let config = CONFIG.init(SunsetMutex::new(config));

    // Read SSH configuration from Flash (if it exists)
    let mut flash_storage = Fl::new(FlashStorage::new(flash::Flash::new()));
    let config = ssh_stamp::storage::load_or_create(&mut flash_storage).await;

    static FLASH: StaticCell<SunsetMutex<Fl>> = StaticCell::new();
    let _flash = FLASH.init(SunsetMutex::new(flash_storage));

    let config = CONFIG.init(SunsetMutex::new(config.unwrap()));

    let wifi_controller = esp_radio::init().unwrap();

    // Bring up the network interface and start accepting SSH connections.
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

    // Initialize the global pin channel and keep the &'static reference so we can
    // pass it to tasks that need to mutate pins (no unsafe globals).
    let pin_channel_ref = pins::init_global_channel(PinChannel::new(
        serde_pin_config,
        available_gpios,
    ));

    // Grab UART1, typically not connected to dev board's TTL2USB IC nor builtin JTAG functionality
    let uart1 = peripherals.UART1;

    // Use the same config reference for UART task.
    // Pass pin_channel_ref into the UART task so it can acquire/release pins.
    interrupt_spawner
        .spawn(uart_task(uart_buf, uart1, pin_channel_ref))
        .unwrap();

    // Pass pin_channel_ref into accept_requests (so SSH handlers can use it).
    // NOTE: accept_requests signature must accept this arg; if it doesn't,
    // thread the reference into whatever code spawns handle_ssh_client.
    accept_requests(tcp_stack, uart_buf, pin_channel_ref).await;
}

static UART_BUF: StaticCell<BufferedUart> = StaticCell::new();
static INT_EXECUTOR: StaticCell<InterruptExecutor<0>> = StaticCell::new();

#[embassy_executor::task()]
async fn uart_task(
    buffer: &'static BufferedUart,
    uart_periph: UART1<'static>,
    pin_channel_ref: &'static SunsetMutex<PinChannel>,
) {
    dbg!("Spawning UART task...");
    // Hardware UART setup
    let uart_config = Config::default().with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1),
    );

    // Use the pinned reference passed in from main.
    let mut pin_chan = pin_channel_ref.lock().await;

    // Sync pin config via channels
    pin_chan
        .with_channel(async |rx, tx| {
            let uart = Uart::new(uart_periph, uart_config)
                .unwrap()
                .with_rx(rx)
                .with_tx(tx)
                .into_async();

            // Run the main buffered TX/RX loop
            buffer.run(uart).await;
        })
        .await
        .unwrap();
}
