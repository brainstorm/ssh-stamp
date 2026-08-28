#![feature(prelude_import)]
//! ESP32-family `ssh-stamp` binary.
//!
//! Brings up ESP-specific peripherals (heap, flash, RNG, UART, radio), then
//! hands control to the platform-agnostic [`ssh_stamp::app::run_app`].
//!
//! # Pin assignments
//!
//! UART, CAN and I2C pin numbers are defined per-board in `boards/*.toml`
//! files in the `ssh-stamp-esp32-boards` crate. Select a board via a
//! `board-<name>` feature (e.g. `board-esp32c6-devkitc`). That crate's front
//! page carries the generated catalog: which GPIO each bus uses on each
//! board, and which buses a board does not support yet.
#![no_std]
#![no_main]
extern crate core;
#[prelude_import]
use core::prelude::rust_2024::*;
extern crate alloc;
use embassy_executor::Spawner;
use esp_hal::interrupt::{Priority, software::SoftwareInterruptControl};
use esp_hal::rng::{Trng, TrngSource};
use esp_println::logger;
use esp_rtos::embassy::InterruptExecutor;
use heapless::String;
use log::{debug, error, warn};
use ssh_stamp::config::{SSHStampConfig, UartPins};
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::store;
use ssh_stamp::{
    app, mem_probe::{self, Checkpoint},
    settings::{DEFAULT_IP, HEAP_SIZE},
};
use ssh_stamp_esp32::{
    BufferedUart, EspPlatform, EspUartPins, EspWifi, UART_BUF, bench, flash, mac_address,
    register_custom_rng, uart_task,
};
use ssh_stamp_esp32_boards::Board;
use ssh_stamp_hal::{HalError, WifiError};
use ssh_stamp_hal::{NetworkProviderHal, WifiHal};
use static_cell::StaticCell;
use sunset_async::SunsetMutex;
static INT_EXECUTOR: StaticCell<InterruptExecutor<1>> = StaticCell::new();
#[doc(hidden)]
pub(crate) mod __main {
    use super::*;
    #[doc(hidden)]
    fn ____embassy_main_task(
        spawner: Spawner,
    ) -> impl ::core::future::Future<Output = ::embassy_executor::_export::Never> {
        #[doc(hidden)]
        async fn ____embassy_main_task_inner_function(spawner: Spawner) -> ! {
            {
                {
                    static mut HEAP: core::mem::MaybeUninit<[u8; HEAP_SIZE]> = core::mem::MaybeUninit::uninit();
                    unsafe {
                        ::esp_alloc::HEAP
                            .add_region(
                                ::esp_alloc::HeapRegion::new(
                                    HEAP.as_mut_ptr() as *mut u8,
                                    HEAP_SIZE,
                                    ::esp_alloc::MemoryCapability::Internal.into(),
                                ),
                            );
                    }
                };
                #[unsafe(export_name = "esp_app_desc")]
                #[unsafe(link_section = ".flash.appdesc")]
                #[used]
                /// Application metadata descriptor.
                pub static ESP_APP_DESC: ::esp_bootloader_esp_idf::EspAppDesc = ::esp_bootloader_esp_idf::EspAppDesc::new_internal(
                    "0.2.0",
                    "ssh-stamp-esp32",
                    ::esp_bootloader_esp_idf::BUILD_TIME,
                    ::esp_bootloader_esp_idf::BUILD_DATE,
                    ::esp_bootloader_esp_idf::ESP_IDF_COMPATIBLE_VERSION,
                    0,
                    u16::MAX,
                    ::esp_bootloader_esp_idf::MMU_PAGE_SIZE,
                    ::esp_bootloader_esp_idf::SECURE_VERSION,
                );
                logger::init_logger_from_env();
                bench::log_heap("boot");
                {
                    {
                        let lvl = ::log::Level::Debug;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("HSM: initialising peripherals"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::__main",
                                    "ssh_stamp_esp32::__main",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                let config = esp_hal::Config::default();
                let peripherals = esp_hal::init(config);
                let trng_source = TrngSource::new(peripherals.RNG, peripherals.ADC1);
                let trng = Trng::try_new()
                    .expect(
                        "TrngSource was just created, so the TRNG must be available",
                    );
                let rng = trng.downgrade();
                register_custom_rng(rng);
                {
                    {
                        let lvl = ::log::Level::Debug;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Initialising flash"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::__main",
                                    "ssh_stamp_esp32::__main",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                flash::init(peripherals.FLASH);
                type B = ::ssh_stamp_esp32_boards::Esp32c6Devkitc;
                {
                    {
                        let lvl = ::log::Level::Debug;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Active board: {0}", B::NAME),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::__main",
                                    "ssh_stamp_esp32::__main",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                let (rx_pin, tx_pin, rx_num, tx_num) = {
                    {
                        {
                            (
                                peripherals.GPIO10.into(),
                                peripherals.GPIO11.into(),
                                10u8,
                                11u8,
                            )
                        }
                    }
                };
                let pins = EspUartPins {
                    rx: rx_pin,
                    tx: tx_pin,
                };
                let uart_pins = UartPins { rx: rx_num, tx: tx_num };
                if true {
                    if !TrngSource::is_enabled() {
                        {
                            ::core::panicking::panic_fmt(
                                format_args!(
                                    "entropy source was disabled before host key generation",
                                ),
                            );
                        }
                    }
                }
                {
                    {
                        let lvl = ::log::Level::Debug;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Loading config"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::__main",
                                    "ssh_stamp_esp32::__main",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                let flash_config = {
                    let Some(flash_storage_guard) = flash::get_flash_n_buffer() else {
                        {
                            ::core::panicking::panic_fmt(
                                format_args!("Could not acquire flash storage lock"),
                            );
                        };
                    };
                    let mut fb = flash_storage_guard.lock().await;
                    let (flash_storage, buf) = fb.split_ref_mut();
                    store::load_or_create(flash_storage, buf, mac_address(), uart_pins)
                }
                    .expect(
                        "Stored config is present but invalid; refusing to overwrite it. \
         Erase the config sector (espflash erase-region 0x9000 0x1000) to reprovision.",
                    );
                let uart_params = flash_config.uart_params;
                static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();
                let config: &'static SunsetMutex<SSHStampConfig> = CONFIG
                    .init(SunsetMutex::new(flash_config));
                {
                    {
                        let lvl = ::log::Level::Debug;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Initialising timers"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::__main",
                                    "ssh_stamp_esp32::__main",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
                use esp_hal::timer::systimer::SystemTimer;
                let systimer = SystemTimer::new(peripherals.SYSTIMER);
                esp_rtos::start(systimer.alarm0, sw_int.software_interrupt0);
                mem_probe::checkpoint(Checkpoint::Boot);
                let uart_buf = UART_BUF.init_with(BufferedUart::new);
                let interrupt_executor = INT_EXECUTOR
                    .init_with(|| InterruptExecutor::new(sw_int.software_interrupt1));
                let interrupt_spawner = interrupt_executor.start(Priority::Priority10);
                interrupt_spawner
                    .spawn(
                        uart_task(uart_buf, peripherals.UART1, pins, uart_params)
                            .expect("uart_task spawn failed"),
                    );
                let platform = EspPlatform::new();
                mem_probe::checkpoint(Checkpoint::PeripheralsReady);
                bench::log_heap("peripherals");
                {
                    {
                        let lvl = ::log::Level::Debug;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("Initialising radio"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::__main",
                                    "ssh_stamp_esp32::__main",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                let ap_config = app::prepare_ap_config(config, &platform)
                    .await
                    .expect("Failed to prepare AP config");
                drop(trng_source);
                let mut wifi = EspWifi::new(spawner, peripherals.WIFI, rng, DEFAULT_IP);
                wifi.configure_ap(ap_config).expect("Failed to configure AP");
                let stack = wifi.bring_up().await;
                match stack {
                    Ok(_) => {}
                    Err(ref e) => {
                        {
                            {
                                let lvl = ::log::Level::Warn;
                                if lvl <= ::log::STATIC_MAX_LEVEL
                                    && lvl <= ::log::max_level()
                                {
                                    ::log::__private_api::log(
                                        { ::log::__private_api::GlobalLogger },
                                        format_args!("Failed to bring up WiFi"),
                                        lvl,
                                        &(
                                            "ssh_stamp_esp32::__main",
                                            "ssh_stamp_esp32::__main",
                                            ::log::__private_api::loc(),
                                        ),
                                        (),
                                    );
                                }
                            }
                        };
                        if let HalError::Wifi(WifiError::StationMode) = e {
                            let mut config_guard = config.lock().await;
                            config_guard.wifi_sta_ssid = String::<32>::new();
                            let _ = platform.save_config(&config_guard).await;
                            {
                                {
                                    let lvl = ::log::Level::Warn;
                                    if lvl <= ::log::STATIC_MAX_LEVEL
                                        && lvl <= ::log::max_level()
                                    {
                                        ::log::__private_api::log(
                                            { ::log::__private_api::GlobalLogger },
                                            format_args!(
                                                "Station Mode failed to connect. Rebooting into Access Point mode...",
                                            ),
                                            lvl,
                                            &(
                                                "ssh_stamp_esp32::__main",
                                                "ssh_stamp_esp32::__main",
                                                ::log::__private_api::loc(),
                                            ),
                                            (),
                                        );
                                    }
                                }
                            };
                            platform.reset();
                        }
                    }
                }
                mem_probe::checkpoint(Checkpoint::WifiUp);
                bench::log_heap("wifi_up");
                if true {
                    if !TrngSource::is_enabled() {
                        {
                            ::core::panicking::panic_fmt(
                                format_args!("no entropy source active after WiFi came up"),
                            );
                        }
                    }
                }
                if let Err(e) = app::run_app(stack.unwrap(), uart_buf, config, &platform)
                    .await
                {
                    {
                        {
                            let lvl = ::log::Level::Error;
                            if lvl <= ::log::STATIC_MAX_LEVEL
                                && lvl <= ::log::max_level()
                            {
                                ::log::__private_api::log(
                                    { ::log::__private_api::GlobalLogger },
                                    format_args!("run_app exited with error: {0}", e),
                                    lvl,
                                    &(
                                        "ssh_stamp_esp32::__main",
                                        "ssh_stamp_esp32::__main",
                                        ::log::__private_api::loc(),
                                    ),
                                    (),
                                );
                            }
                        }
                    };
                }
                {
                    {
                        let lvl = ::log::Level::Warn;
                        if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                            ::log::__private_api::log(
                                { ::log::__private_api::GlobalLogger },
                                format_args!("End of main, resetting"),
                                lvl,
                                &(
                                    "ssh_stamp_esp32::__main",
                                    "ssh_stamp_esp32::__main",
                                    ::log::__private_api::loc(),
                                ),
                                (),
                            );
                        }
                    }
                };
                esp_hal::system::software_reset();
            }
        }
        { ____embassy_main_task_inner_function(spawner) }
    }
    #[doc(hidden)]
    fn __embassy_main(
        spawner: Spawner,
    ) -> ::core::result::Result<
        ::embassy_executor::SpawnToken<impl Sized>,
        ::embassy_executor::SpawnError,
    > {
        const fn __task_pool_get<F, Args, Fut>(
            _: F,
        ) -> &'static ::embassy_executor::raw::TaskPool<Fut, POOL_SIZE>
        where
            F: ::embassy_executor::_export::TaskFn<Args, Fut = Fut>,
            Fut: ::core::future::Future + 'static,
        {
            unsafe { &*POOL.get().cast() }
        }
        const POOL_SIZE: usize = 1;
        static POOL: ::embassy_executor::_export::TaskPoolHolder<
            {
                ::embassy_executor::_export::task_pool_size::<
                    _,
                    _,
                    _,
                    POOL_SIZE,
                >(____embassy_main_task)
            },
            {
                ::embassy_executor::_export::task_pool_align::<
                    _,
                    _,
                    _,
                    POOL_SIZE,
                >(____embassy_main_task)
            },
        > = unsafe {
            ::core::mem::transmute(
                ::embassy_executor::_export::task_pool_new::<
                    _,
                    _,
                    _,
                    POOL_SIZE,
                >(____embassy_main_task),
            )
        };
        unsafe {
            __task_pool_get(____embassy_main_task)
                ._spawn_async_fn(move || ____embassy_main_task(spawner))
        }
    }
    #[doc(hidden)]
    unsafe fn __make_static<T>(t: &mut T) -> &'static mut T {
        ::core::mem::transmute(t)
    }
    #[allow(non_snake_case)]
    #[export_name = "main"]
    ///The main entry point of the firmware, generated by the `#[main]` macro.
    pub fn __risc_v_rt__main() -> ! {
        let mut executor = ::esp_rtos::embassy::Executor::new();
        let executor = unsafe { __make_static(&mut executor) };
        executor
            .run(|spawner| {
                spawner.spawn(__embassy_main(spawner).unwrap());
            })
    }
}
/// `getrandom` custom backend.
///
/// getrandom 0.4 picks its entropy backend by cfg rather than by cargo
/// feature: every bare-metal target here is built with
/// `--cfg getrandom_backend="custom"` (see `.cargo/config.toml`), which
/// makes getrandom link this symbol. It must be defined exactly once in the
/// whole program, so it lives in the binary — as getrandom's own docs
/// recommend — and forwards to the hardware TRNG registered at boot by
/// [`register_custom_rng`].
///
/// This is what feeds the SSH host key and the `WiFi` PSK, so it must be a
/// real entropy source, never a stub.
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    let buf = unsafe {
        core::ptr::write_bytes(dest, 0, len);
        core::slice::from_raw_parts_mut(dest, len)
    };
    ssh_stamp_esp32::rng_fill_bytes(buf)
}
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
