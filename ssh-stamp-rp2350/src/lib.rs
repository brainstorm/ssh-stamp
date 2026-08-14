// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! RP2350 port of `ssh-stamp`, for the `WIZnet` [W6300-EVB-Pico2].
//!
//! Unlike the ESP32 ports there is no radio here: the network comes up over
//! wired Ethernet through the onboard W6300, so there is no AP fallback and
//! no `WifiHal` implementation — only [`ssh_stamp_hal::NetworkProviderHal`].
//!
//! [W6300-EVB-Pico2]: https://docs.wiznet.io/Product/Chip/Ethernet/W6300/w6300-evb-pico2
//!
//! # Board pinout
//!
//! Pin numbers come from the `ssh-stamp-rp2350-boards` BSP crate, same as
//! the ESP32 port takes its pins from `ssh-stamp-esp32-boards`: each PCB is
//! a `boards/*.toml`, and `build.rs` generates the `take_uart_pins!` /
//! `take_ethernet_pins!` / `select_board!` macros the binary uses. Nothing
//! in this crate hard-codes a GPIO number.
//!
//! For the W6300-EVB-Pico2 that resolves to UART0 on GPIO0/1, and the
//! W6300 on GPIO15-22 (IO2/IO3 unused — single-SPI only, see [`net`]).

#![no_std]
#![forbid(unsafe_code)]

pub mod flash;
pub mod net;
pub mod platform;
pub mod rng;
pub mod uart;

pub use flash::{ConfigFlash, FlashBuffer, get_flash_n_buffer};
pub use net::{W6300Ethernet, W6300Spi};
pub use platform::{Rp2350OtaWriter, Rp2350Platform};
pub use rng::{entropy_task, pool_level, prime_pool};
pub use uart::{BufferedUart, UART_BUF, UART_SIGNAL, uart_task};
