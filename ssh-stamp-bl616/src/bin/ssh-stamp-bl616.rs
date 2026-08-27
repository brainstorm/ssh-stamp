// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! ssh-stamp on the Sipeed M0S Dock (Bouffalo BL616).
//!
//! # Startup order, which is not the ESP one
//!
//! `FreeRTOS` owns `main`. `bl616_wifi::main!` runs `board_init()`, brings the
//! radio up *inside a task* — doing it before the scheduler starts resets the
//! chip — and starts the scheduler. Only then does an embassy executor exist,
//! hosted as one `FreeRTOS` task by [`bl616_wifi::embassy_rt::run`], and
//! ssh-stamp's async entry is spawned there.
//!
//! So there is no `#[embassy_executor::main]` here, and there cannot be.

#![no_std]
#![no_main]

use bl616_wifi::{main, println};
use embassy_futures::block_on;
use ssh_stamp::app;
use ssh_stamp::config::SSHStampConfig;
use ssh_stamp_bl616::{
    Bl616Platform, Bl616Serial, Bl616Wifi, DEFAULT_UART_PINS, UART_BUF, load_config,
    rng_fill_bytes, uart_task,
};
use ssh_stamp_hal::NetworkProviderHal;
use static_cell::StaticCell;
use sunset_async::SunsetMutex;

// 64 KiB. `run_app` puts a 8 KiB receive buffer and a 4 KiB transmit buffer
// on the stack before sunset's own frames, so the 8 KiB default overruns the
// moment an SSH session is served -- silently, taking the radio with it a
// little later.
main!(app, stack = 16 * 1024);

fn app() -> ! {
    println!("[ssh-stamp] bl616 starting");

    // Everything that talks to the vendor stack happens here, before the
    // executor exists. Those calls block on FreeRTOS primitives, and blocking
    // inside executor.poll() hangs the board with no timeout -- found the
    // hard way, twice.
    let radio = match Bl616Wifi::init_radio() {
        Ok(w) => w,
        Err(e) => {
            println!("[ssh-stamp] radio did not come up: {e:?}");
            halt();
        }
    };
    let mac = radio.sta_mac();
    println!("[ssh-stamp] radio ready, mac {mac:02x?}");

    let stored = match block_on(load_config(mac, DEFAULT_UART_PINS)) {
        Ok(c) => c,
        Err(e) => {
            // Refusing beats recreating: a new config would regenerate the
            // host key, breaking client pinning and reopening the
            // unauthenticated first-login window.
            println!("[ssh-stamp] stored config invalid ({e:?}); erase the config sector");
            halt();
        }
    };
    let config: &'static SunsetMutex<SSHStampConfig> = CONFIG.init(SunsetMutex::new(stored));
    let platform = Bl616Platform::new();

    let ap_config = match block_on(app::prepare_ap_config(config, &platform)) {
        Ok(c) => c,
        Err(e) => {
            println!("[ssh-stamp] could not prepare the AP config: {e:?}");
            halt();
        }
    };
    // Print both. The password is generated on first boot and stored, so
    // without this a headless board is unreachable: there is nowhere else to
    // read it from. `ssh-stamp` logs it through `info!`, which needs a logger
    // this port does not install.
    println!(
        "[ssh-stamp] ssid {:?} psk {:?}",
        ap_config.ap_ssid.as_str(),
        ap_config.ap_password.as_str()
    );

    // The last vendor call, and still before the executor.
    let address = match Bl616Wifi::start_radio(&radio, &ap_config) {
        Ok(a) => a,
        Err(e) => {
            println!("[ssh-stamp] radio would not start: {e:?}");
            halt();
        }
    };
    println!("[ssh-stamp] radio started");

    bl616_wifi::embassy_rt::run(move |spawner| {
        let mut wifi = Bl616Wifi::new(radio, spawner);
        wifi.adopt(ap_config, address);

        let serial: &'static Bl616Serial = UART_BUF.init(Bl616Serial::new());
        spawner.spawn(
            uart_task(serial, bl616_wifi::uart::Config::default()).expect("task pool exhausted"),
        );
        spawner.spawn(run(wifi, serial, config).expect("task pool exhausted"));
    })
}

/// Stop, without returning from a `-> !` function.
fn halt() -> ! {
    loop {
        bl616_wifi::delay_ms(1_000);
    }
}

// Not yet awaiting anything: the app loop that will is the next milestone.
#[allow(clippy::unused_async)]
/// The live configuration, shared with every SSH session.
static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();

#[embassy_executor::task]
async fn run(
    mut wifi: Bl616Wifi,
    serial: &'static Bl616Serial,
    config: &'static SunsetMutex<SSHStampConfig>,
) {
    println!("[ssh-stamp] building the network stack");
    let stack = match wifi.bring_up().await {
        Ok(s) => s,
        Err(e) => {
            println!("[ssh-stamp] network did not come up: {e:?}");
            return;
        }
    };
    println!("[ssh-stamp] network up; ssh on port 22");

    let platform = Bl616Platform::new();
    if let Err(e) = app::run_app(stack, serial, config, &platform).await {
        println!("[ssh-stamp] run_app exited: {e:?}");
    }
}

/// `getrandom`'s custom backend, defined exactly once per binary.
///
/// sunset draws fresh key-exchange material per connection, so this is on the
/// path of every SSH handshake, not just boot-time key minting.
///
/// # Safety
///
/// Called by `getrandom` with a valid `dest` for `len` bytes.
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    let buf = unsafe {
        core::ptr::write_bytes(dest, 0, len);
        core::slice::from_raw_parts_mut(dest, len)
    };
    rng_fill_bytes(buf).map_err(|_| getrandom::Error::UNEXPECTED)
}
