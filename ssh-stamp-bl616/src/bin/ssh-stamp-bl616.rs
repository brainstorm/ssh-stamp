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
use ssh_stamp_bl616::{Bl616Serial, Bl616Wifi, UART_BUF, rng_fill_bytes, uart_task};

main!(app);

fn app() -> ! {
    println!("[ssh-stamp] bl616 starting");

    bl616_wifi::embassy_rt::run(|spawner| {
        spawner.spawn(run(spawner).expect("task pool exhausted"));
    })
}

// Not yet awaiting anything: the app loop that will is the next milestone.
#[allow(clippy::unused_async)]
#[embassy_executor::task]
async fn run(spawner: Spawner) {
    let wifi = match Bl616Wifi::new(spawner) {
        Ok(w) => w,
        Err(e) => {
            println!("[ssh-stamp] radio unavailable: {e:?}");
            return;
        }
    };
    println!("[ssh-stamp] mac {:02x?}", wifi.mac());

    // The serial bridge waits on UART_SIGNAL, so opening UART0 costs nothing
    // until a session actually attaches.
    let serial: &'static Bl616Serial = UART_BUF.init(Bl616Serial::new());
    spawner.spawn(
        uart_task(serial, bl616_wifi::uart::Config::default()).expect("task pool exhausted"),
    );

    // The rest — prepare_ap_config, store::load_or_create, run_app — is the
    // next milestone. See the crate docs for what is deliberately not
    // implemented yet.
    println!("[ssh-stamp] radio up, serial bridge armed");
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
