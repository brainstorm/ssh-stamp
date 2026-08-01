// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `ssh-stamp` firmware for the `WIZnet` W6300-EVB-Pico2 (RP2350 + W6300).
//!
//! Boot flow: peripherals → RNG → config from flash → UART bridge task →
//! W6300 Ethernet (DHCP) → [`ssh_stamp::app::run_app`].
//!
//! Logs go out over the Pico 2's built-in USB as a CDC serial port
//! (`/dev/ttyACM0`), so no debug probe or TTL adapter is needed. That
//! matters here: with no `WiFi` AP to fall back on, USB is the only way to
//! see why the network did not come up.

#![no_std]
#![no_main]

extern crate alloc;

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::{DMA_CH0, DMA_CH1, PIO0, TRNG, UART0, USB};
use embassy_rp::pio::{InterruptHandler as PioInterruptHandler, Pio};
use embassy_rp::pio_programs::spi::Spi as PioSpi;
use embassy_rp::trng::{InterruptHandler as TrngInterruptHandler, Trng};
use embassy_rp::uart::{BufferedInterruptHandler, BufferedUart as RpBufferedUart};
use embassy_rp::usb::{Driver as UsbDriver, InterruptHandler as UsbInterruptHandler};
use embassy_rp::{dma, trng};
use embassy_time::{Delay, Timer};
use embedded_alloc::LlffHeap as Heap;
use embedded_hal_bus::spi::ExclusiveDevice;
use log::{debug, error, info};
use ssh_stamp::config::{SSHStampConfig, UartPins};
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::{app, store};
use ssh_stamp_hal::NetworkProviderHal;
use ssh_stamp_rp2350::{
    BufferedUart, Rp2350Platform, UART_BUF, W6300Ethernet, flash, net, register_custom_rng,
    uart_task,
};
use ssh_stamp_rp2350_boards::Board;
use static_cell::StaticCell;
use sunset_async::SunsetMutex;

/// sunset, ed25519-dalek and the executor all pull in `alloc`.
#[global_allocator]
static HEAP: Heap = Heap::empty();
const HEAP_SIZE: usize = 64 * 1024;
static mut HEAP_MEM: [core::mem::MaybeUninit<u8>; HEAP_SIZE] =
    [core::mem::MaybeUninit::uninit(); HEAP_SIZE];

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
    UART0_IRQ => BufferedInterruptHandler<UART0>;
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    TRNG_IRQ => TrngInterruptHandler<TRNG>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
});

/// USB CDC logger: `log::*` shows up on /dev/ttyACM0.
#[embassy_executor::task]
async fn logger_task(driver: UsbDriver<'static, USB>) {
    embassy_usb_logger::run!(4096, log::LevelFilter::Info, driver);
}

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    let p = embassy_rp::init(embassy_rp::config::Config::default());

    {
        // SAFETY: called exactly once, before any allocation.
        #[allow(unsafe_code)]
        unsafe {
            HEAP.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE);
        }
    }

    spawner.spawn(logger_task(UsbDriver::new(p.USB, Irqs)).expect("logger task spawn failed"));
    // Give the host a moment to enumerate the CDC port, otherwise the
    // early boot lines are lost.
    Timer::after_secs(2).await;
    info!("ssh-stamp rp2350: booting (W6300-EVB-Pico2)");

    let mut trng = Trng::new(p.TRNG, Irqs, trng::Config::default());
    let seed = trng.blocking_next_u64();
    let mac = mac_address(&mut trng);
    register_custom_rng(trng);

    // Board selection — the generated select_board! macro expands to the
    // active board's struct as B. Pin numbers come from boards/*.toml via
    // build.rs codegen; no per-board lines here.
    ssh_stamp_rp2350_boards::select_board!();
    debug!("Active board: {}", B::NAME);

    debug!("Initialising config flash");
    flash::init(p.FLASH);

    let (uart_rx_pin, uart_tx_pin, uart_rx_num, uart_tx_num) =
        ssh_stamp_rp2350_boards::take_uart_pins!(p);
    let uart_pins = UartPins {
        rx: uart_rx_num,
        tx: uart_tx_num,
    };

    debug!("Loading config");
    let flash_config = {
        let Some(guard) = flash::get_flash_n_buffer() else {
            panic!("Could not acquire flash storage lock");
        };
        let mut view = guard.lock().await;
        let (storage, buf) = view.split_ref_mut();
        store::load_or_create(storage, buf, mac, uart_pins)
    }
    .expect("Could not load or create SSHStampConfig");

    static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();
    let config: &'static SunsetMutex<SSHStampConfig> = CONFIG.init(SunsetMutex::new(flash_config));

    // The bridged UART: GPIO0/1 at 115200 8N1.
    static TX_BUF: StaticCell<[u8; 256]> = StaticCell::new();
    static RX_BUF: StaticCell<[u8; 256]> = StaticCell::new();
    let uart = RpBufferedUart::new(
        p.UART0,
        uart_tx_pin,
        uart_rx_pin,
        Irqs,
        TX_BUF.init([0u8; 256]),
        RX_BUF.init([0u8; 256]),
        embassy_rp::uart::Config::default(),
    );

    let uart_buf = UART_BUF.init(BufferedUart::new());
    spawner.spawn(uart_task(uart_buf, uart).expect("uart task spawn failed"));

    // W6300 over PIO SPI (the board's pins do not match hardware SPI0).
    debug!("Initialising W6300 over PIO SPI");
    let (int_pin, cs_pin, sck_pin, io0_pin, io1_pin, rst_pin) =
        ssh_stamp_rp2350_boards::take_ethernet_pins!(p);

    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi_bus = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        sck_pin,
        io0_pin,
        io1_pin,
        p.DMA_CH0,
        p.DMA_CH1,
        Irqs,
        net::spi_config(),
    );
    let cs = Output::new(cs_pin, Level::High);
    let spi = ExclusiveDevice::new(spi_bus, cs, Delay).expect("CS pin accepted");

    let int = Input::new(int_pin, Pull::Up);
    let reset = Output::new(rst_pin, Level::High);

    let platform = Rp2350Platform::new();
    {
        let guard = config.lock().await;
        app::print_hostkey_fingerprint(&guard.hostkey);
    }

    let mut ethernet = W6300Ethernet::new(spawner, mac, seed, spi, int, reset);
    let stack = match ethernet.bring_up().await {
        Ok(stack) => stack,
        Err(e) => {
            error!("Ethernet bring-up failed: {e:?}");
            error!("Check the RJ45 cable, then SPI wiring/clock (see net.rs)");
            platform.reset();
        }
    };

    if let Err(e) = app::run_app(stack, uart_buf, config, &platform).await {
        error!("run_app exited with error: {e}");
    }

    error!("End of main, resetting");
    platform.reset();
}

/// Derive a stable locally-administered MAC.
///
/// The W6300 has no MAC in eFuse, so the first boot mints one at random and
/// `store` persists it alongside the rest of the config; later boots load
/// that value and never reach this default.
fn mac_address(trng: &mut Trng<'static, TRNG>) -> [u8; 6] {
    let mut mac = [0u8; 6];
    trng.blocking_fill_bytes(&mut mac);
    // Locally administered, unicast.
    mac[0] = (mac[0] | 0b0000_0010) & 0b1111_1110;
    mac
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
