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
use embassy_usb_logger::ReceiverHandler as _;
use embedded_alloc::LlffHeap as Heap;
use embedded_hal_bus::spi::ExclusiveDevice;
use log::{debug, error, info};
use ssh_stamp::config::{SSHStampConfig, UartPins};
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::{app, store};
use ssh_stamp_hal::NetworkProviderHal;
use ssh_stamp_rp2350::{
    BufferedUart, Rp2350Platform, UART_BUF, W6300Ethernet, flash, net, rng, uart_task,
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

/// Liveness beacon: blinks the onboard LED and logs a counter.
///
/// Two jobs. The LED shows the executor is still scheduling even when the
/// USB log is unreadable, which is the difference between "wedged" and
/// "waiting". The log line repeats forever, so a host that attaches late
/// still sees output — the boot banner alone is missed by anyone who does
/// not have the port open within the first couple of seconds.
#[embassy_executor::task]
async fn heartbeat_task(mut led: Output<'static>) {
    let mut n: u32 = 0;
    loop {
        led.toggle();
        // The LED is the point; logging every tick just floods the console.
        debug!("alive {n}");
        n = n.wrapping_add(1);
        Timer::after_secs(2).await;
    }
}

/// Reboots into the ROM bootloader when the host sends `b`.
///
/// The RP2350 has no reset line exposed over USB, so reflashing otherwise
/// means physically holding BOOTSEL while power-cycling. During bring-up
/// that is the slowest step in the loop by a wide margin. `reset_to_usb_boot`
/// is the same call `picotool reboot -f -u` relies on.
///
/// Only reachable by whoever already has the USB cable, which is the same
/// person who could press the button.
struct BootselOnB;

impl embassy_usb_logger::ReceiverHandler for BootselOnB {
    async fn handle_data(&self, data: &[u8]) {
        if data.contains(&b'b') {
            info!("host asked for BOOTSEL, rebooting into the bootloader");
            // Give the line a moment to reach the host before the USB
            // device disappears.
            Timer::after_millis(100).await;
            embassy_rp::rom_data::reset_to_usb_boot(0, 0);
        }
    }

    fn new() -> Self {
        Self
    }
}

/// USB CDC logger: `log::*` shows up on /dev/ttyACM0.
///
/// Also accepts `b` on the same port to reboot into the bootloader; see
/// [`BootselOnB`].
#[embassy_executor::task]
async fn logger_task(driver: UsbDriver<'static, USB>) {
    embassy_usb_logger::run!(4096, log::LevelFilter::Debug, driver, BootselOnB);
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

    // GPIO25 is the Pico 2's onboard LED and is not otherwise claimed by
    // this board (UART uses 0/1, the W6300 uses 15-19 and 22).
    spawner.spawn(heartbeat_task(Output::new(p.PIN_25, Level::Low)).expect("heartbeat spawn"));

    // A longer inverter chain than the default `One`: the ring oscillator
    // runs slower but jitters more per sample, which is what the hardware
    // autocorrelation test wants. At the default this board failed that test
    // constantly, and each failure costs a soft reset and re-init, so
    // measured throughput was ~2.4 bytes/second — unusable for SSH.
    let mut trng_config = trng::Config::default();
    trng_config.inverter_chain_length = trng::InverterChainLength::Four;
    let trng = Trng::new(p.TRNG, Irqs, trng_config);

    // The pool task is the TRNG's only owner, from here to power-off: every
    // other path into embassy-rp's driver blocks without yielding, which on
    // a cooperative executor starves USB along with everything else. See
    // `rng`'s module docs.
    spawner.spawn(rng::entropy_task(trng).expect("entropy task spawn failed"));

    // Bank entropy before anything can consume it, so the host key and the
    // first key exchange are not drawn from a nearly-empty pool.
    rng::prime_pool().await;

    let mut seed_bytes = [0u8; 8];
    if rng::fill_bytes(&mut seed_bytes).is_err() {
        error!("no entropy for the network stack seed; continuing with zeros");
    }
    let seed = u64::from_le_bytes(seed_bytes);

    let mac = mac_address();
    info!(
        "MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

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

    // The persisted MAC, not the one just generated. `mac` is only the
    // first-boot default handed to `load_or_create`; on every later boot the
    // stored value is what the config holds, and using the generated one
    // would give the board a different address on each power cycle —
    // churning ARP entries and DHCP leases for no reason. It also keeps the
    // address stable when the TRNG comes up cold and the default is zeros.
    let stored_mac = { config.lock().await.mac };
    let mut ethernet = W6300Ethernet::new(spawner, stored_mac, seed, spi, int, reset);
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

/// `getrandom` custom backend.
///
/// getrandom 0.4 picks its entropy backend by cfg rather than by cargo
/// feature: this target is built with `--cfg getrandom_backend="custom"`
/// (see `.cargo/config.toml`), which makes getrandom link this symbol. It
/// must be defined exactly once in the whole program, so it lives in the
/// binary — as getrandom's own docs recommend — and forwards to the pool
/// that [`entropy_task`] keeps filled from the TRNG.
///
/// This is what feeds the SSH host key, so it must be a real entropy
/// source, never a stub.
#[unsafe(no_mangle)]
#[allow(unsafe_code)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    // SAFETY: getrandom guarantees `dest` is valid for writes of `len`
    // bytes. The buffer may be uninitialised, so it is zeroed before a
    // slice is formed over it, as getrandom's documentation prescribes.
    let buf = unsafe {
        core::ptr::write_bytes(dest, 0, len);
        core::slice::from_raw_parts_mut(dest, len)
    };
    ssh_stamp_rp2350::rng_fill_bytes(buf)
}

/// Mint a locally-administered MAC.
///
/// The W6300 has no MAC in eFuse, so the first boot draws one from the
/// entropy pool and `store` persists it alongside the rest of the config;
/// later boots load that value and never reach this default.
fn mac_address() -> [u8; 6] {
    let mut mac = [0u8; 6];
    // A zero MAC from an exhausted pool is still a working (if unlovely)
    // locally-administered address, and only ever reaches flash on a first
    // boot that already logged the shortfall.
    if rng::fill_bytes(&mut mac).is_err() {
        error!("no entropy for a MAC address");
    }
    // Locally administered, unicast.
    mac[0] = (mac[0] | 0b0000_0010) & 0b1111_1110;
    mac
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // Reset rather than spin: a silent `loop {}` is indistinguishable from a
    // hang, since USB stays enumerated when a device merely goes quiet. A
    // reboot is at least something the host can see.
    //
    // Deliberately does not log. The logger writes through a pipe guarded by
    // `critical_section`, which is not reentrant: if the panic happened
    // while that guard was held, logging from here deadlocks — the one
    // outcome a panic handler must not produce. Send `b` on the USB console
    // to drop into BOOTSEL if a boot loop needs interrupting.
    let _ = info;
    cortex_m::peripheral::SCB::sys_reset()
}
