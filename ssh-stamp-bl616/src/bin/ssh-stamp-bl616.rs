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
use embassy_executor::Spawner;
use ssh_stamp::app;
use ssh_stamp::config::SSHStampConfig;
use ssh_stamp_bl616::{
    Bl616Platform, Bl616Serial, Bl616Wifi, DEFAULT_UART_PINS, UART_BUF, load_config,
    rng_fill_bytes, uart_task,
};
use ssh_stamp_hal::{NetworkProviderHal, WifiHal};
use static_cell::StaticCell;
use sunset_async::SunsetMutex;

main!(app);

fn app() -> ! {
    println!("[ssh-stamp] bl616 starting");

    bl616_wifi::embassy_rt::run(|spawner| {
        spawner.spawn(run(spawner).expect("task pool exhausted"));
    })
}

// Not yet awaiting anything: the app loop that will is the next milestone.
#[allow(clippy::unused_async)]
/// The live configuration, shared with every SSH session.
static CONFIG: StaticCell<SunsetMutex<SSHStampConfig>> = StaticCell::new();

#[embassy_executor::task]
async fn run(spawner: Spawner) {
    let mut wifi = match Bl616Wifi::new(spawner) {
        Ok(w) => w,
        Err(e) => {
            println!("[ssh-stamp] radio unavailable: {e:?}");
            return;
        }
    };
    let mac = wifi.mac();
    println!("[ssh-stamp] mac {mac:02x?}");

    // The serial bridge waits on UART_SIGNAL, so opening UART0 costs nothing
    // until a session actually attaches.
    let serial: &'static Bl616Serial = UART_BUF.init(Bl616Serial::new());
    spawner.spawn(
        uart_task(serial, bl616_wifi::uart::Config::default()).expect("task pool exhausted"),
    );

    // Refusing to boot on a corrupt config is the safe side of the trade:
    // recreating one would regenerate the SSH host key, breaking client
    // host-key pinning and reopening the unauthenticated first-login window.
    let stored = match load_config(mac, DEFAULT_UART_PINS).await {
        Ok(c) => c,
        Err(e) => {
            println!(
                "[ssh-stamp] stored config is present but invalid ({e:?}); \
                 refusing to overwrite it. Erase the config sector to reprovision."
            );
            return;
        }
    };

    let config: &'static SunsetMutex<SSHStampConfig> = CONFIG.init(SunsetMutex::new(stored));

    let platform = Bl616Platform::new();

    // Mints the WiFi password if the stored config did not carry one, so it
    // draws on the TRNG before the radio starts.
    let ap_config = match app::prepare_ap_config(config, &platform).await {
        Ok(c) => c,
        Err(e) => {
            println!("[ssh-stamp] could not prepare the AP config: {e:?}");
            return;
        }
    };

    if let Err(e) = wifi.configure_ap(ap_config) {
        println!("[ssh-stamp] AP config rejected: {e:?}");
        return;
    }

    let stack = match wifi.bring_up().await {
        Ok(s) => s,
        Err(e) => {
            println!("[ssh-stamp] network did not come up: {e:?}");
            return;
        }
    };
    println!("[ssh-stamp] network up; listening for ssh on port 22");

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
