// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! ssh-stamp on the Bouffalo BL616.
//!
//! The port implements `ssh-stamp-hal` over [`bl616-wifi`], which drives the
//! vendor's `WiFi` blobs and presents the MAC as an `embassy_net_driver::Driver`.
//!
//! # What owns what
//!
//! Three things may exist only once in a binary, and on this port all three
//! come from `bl616-wifi`, not from here and not from `ssh-stamp`:
//!
//! * the **global allocator**, over the SDK's heap;
//! * the **`critical-section` implementation**, over `mstatus.MIE`;
//! * the **embassy time driver**, over the `FreeRTOS` tick.
//!
//! So this crate must not pull in `embedded-alloc`, must not enable
//! `riscv/critical-section-single-hart`, and must not enable
//! `embassy-executor`'s `platform-*` features — the last of those would
//! supply a second `__pender`, and the stock RISC-V executor parks the hart
//! in `wfi`, which is wrong inside a `FreeRTOS` task.
//!
//! # Startup order
//!
//! `FreeRTOS` owns `main`. `bl616_wifi::main!` runs `board_init()`, brings the
//! radio up *inside a task* (doing it before the scheduler starts resets the
//! chip) and starts the scheduler; the application then calls
//! [`bl616_wifi::embassy_rt::run`], which hosts an embassy executor as one
//! `FreeRTOS` task. ssh-stamp's async entry is spawned there.

#![no_std]

extern crate alloc;

pub mod flash;
pub mod hash;
pub mod network;
pub mod platform;
pub mod rng;
pub mod timer;
pub mod uart;

pub use flash::Bl616Flash;
pub use hash::Bl616Hash;
pub use network::{Bl616Wifi, net_up};
pub use platform::{Bl616OtaWriter, Bl616Platform};
pub use rng::{Bl616Rng, fill_bytes as rng_fill_bytes};
pub use timer::Bl616Timer;
pub use uart::{Bl616Serial, UART_BUF, UART_SIGNAL, uart_task};

/// The station MAC, which ssh-stamp uses to name the default network.
///
/// Takes the radio handle rather than reading efuse directly: the vendor
/// manager owns the address and may have been told to override it.
#[must_use]
pub fn mac_address(wifi: &Bl616Wifi) -> [u8; 6] {
    wifi.mac()
}
