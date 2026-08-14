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
use embassy_time::{Delay, Timer, with_timeout};
use embedded_alloc::LlffHeap as Heap;
use embedded_hal_bus::spi::ExclusiveDevice;
use embassy_usb_logger::ReceiverHandler as _;
use log::{debug, error, info, warn};
use ssh_stamp::config::{SSHStampConfig, UartPins};
use ssh_stamp::platform::PlatformServices;
use ssh_stamp::{app, store};
use ssh_stamp_hal::NetworkProviderHal;
use ssh_stamp_rp2350::{
    BufferedUart, Rp2350Platform, UART_BUF, W6300Ethernet, entropy_task, flash, net, prime_pool,
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

/// How long to wait for one TRNG generation attempt.
const TRNG_TIMEOUT: embassy_time::Duration = embassy_time::Duration::from_secs(3);
/// How many attempts before declaring the TRNG dead. The first one after
/// power-on usually fails while the ring oscillator settles.
const TRNG_ATTEMPTS: u32 = 5;

/// Entropy to bank before starting anything that consumes it.
const ENTROPY_PRIME_BYTES: usize = 256;
/// Ceiling on the boot-time entropy wait.
const ENTROPY_PRIME_TIMEOUT: embassy_time::Duration = embassy_time::Duration::from_secs(30);

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


    // Async, not `blocking_next_u64()`/`blocking_fill_bytes()`. embassy-rp's
    // blocking TRNG path is
    //
    //     while trng_busy_register.read().trng_busy() {}
    //
    // with no yield and no timeout, so on a cooperative executor a TRNG that
    // never finishes a generation starves every other task — USB included,
    // which is what made this look like a dead board rather than a stuck
    // call. The async version waits on TRNG_IRQ and yields, so the rest of
    // the firmware keeps running and a stall is visible instead of fatal.
    // A longer inverter chain than the default `One`. The ring oscillator
    // runs slower with more jitter per sample, which is what the hardware
    // autocorrelation test wants: with the default this board failed that
    // test constantly, and each failure costs a soft reset and re-init, so
    // measured throughput was ~2.4 bytes/second — unusable for SSH.
    let mut trng_config = trng::Config::default();
    trng_config.inverter_chain_length = trng::InverterChainLength::Four;
    let mut trng = Trng::new(p.TRNG, Irqs, trng_config);

    // Retried, because the first generation after power-on routinely takes
    // far longer than later ones: the hardware fails its autocorrelation
    // test a few times while the ring oscillator settles, and each failure
    // costs a soft reset and re-init inside the driver. A single short
    // attempt reports weak entropy on a chip that is merely still warming
    // up, and weak entropy is not something an SSH host key should be
    // built on.
    let mut seed_bytes = [0u8; 8];
    let mut entropy_ok = false;
    for attempt in 1..=TRNG_ATTEMPTS {
        if with_timeout(TRNG_TIMEOUT, trng.fill_bytes(&mut seed_bytes))
            .await
            .is_ok()
        {
            entropy_ok = true;
            debug!(
                "TRNG ready after {attempt} attempt(s), {}ms",
                embassy_time::Instant::now().as_millis()
            );
            break;
        }
        warn!("TRNG attempt {attempt}/{TRNG_ATTEMPTS} produced nothing in {}ms",
            TRNG_TIMEOUT.as_millis());
    }
    if !entropy_ok {
        error!(
            "TRNG never produced entropy. Keys generated this boot are NOT \
             safe; re-flash or replace the board before trusting it."
        );
    }
    let seed = u64::from_le_bytes(seed_bytes);
    debug!("TRNG ok={entropy_ok}");

    let mac = mac_address(&mut trng).await;
    info!(
        "MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5]
    );
    // The TRNG is owned by the pool task from here on; nothing else may
    // touch it, since every other access path into embassy-rp's driver is
    // a blocking one. See `rng`'s module docs.
    spawner.spawn(entropy_task(trng).expect("entropy task spawn failed"));

    // Wait for real entropy before anything can consume it. A key exchange
    // needs more than one TRNG generation, so bringing the SSH server up
    // against a nearly-empty pool gives handshakes that die at userauth
    // with no explanation on either side.
    let level = prime_pool(ENTROPY_PRIME_BYTES, ENTROPY_PRIME_TIMEOUT).await;
    if level < ENTROPY_PRIME_BYTES {
        warn!("entropy pool only reached {level}/{ENTROPY_PRIME_BYTES} bytes; SSH may fail");
    } else {
        info!("entropy pool ready ({level} bytes)");
    }

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

/// Derive a stable locally-administered MAC.
///
/// The W6300 has no MAC in eFuse, so the first boot mints one at random and
/// `store` persists it alongside the rest of the config; later boots load
/// that value and never reach this default.
async fn mac_address(trng: &mut Trng<'static, TRNG>) -> [u8; 6] {
    let mut mac = [0u8; 6];
    // Async for the same reason as the seed above; a zero MAC from a failed
    // read is still a working (if unlovely) locally-administered address.
    if with_timeout(TRNG_TIMEOUT, trng.fill_bytes(&mut mac)).await.is_err() {
        error!("TRNG stalled while generating a MAC");
    }
    // Locally administered, unicast.
    mac[0] = (mac[0] | 0b0000_0010) & 0b1111_1110;
    mac
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // A silent `loop {}` here is indistinguishable from a hang: the board
    // stops, USB stays enumerated because the host does not notice a device
    // that merely went quiet, and nothing says why. Reset instead, so a
    // panic shows up as a reboot the host can see (and logs restart), and
    // try to say what happened on the way out.
    // Deliberately does not log. The logger writes through a pipe guarded
    // by `critical_section`, which is not reentrant: if the panic happened
    // while that guard was held — anywhere inside the logging path, or in
    // any other critical section — logging from here deadlocks and the
    // board hangs instead of resetting. A hang is the one outcome a panic
    // handler must not produce, because it is indistinguishable from the
    // firmware simply stopping.
    let _ = info;

    // Spin before resetting so a panic during early boot does not reboot so
    // fast that the board is hard to catch in BOOTSEL.
    for _ in 0..20_000_000 {
        core::hint::spin_loop();
    }
    cortex_m::peripheral::SCB::sys_reset()
}
