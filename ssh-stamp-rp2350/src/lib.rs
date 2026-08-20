// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! RP2350 port of `ssh-stamp`, for the `WIZnet` [W6300-EVB-Pico2].
//!
//! The network comes up over wired Ethernet through the onboard W6300, so
//! this port implements [`ssh_stamp_hal::NetworkProviderHal`] and no radio
//! trait: there is no access point to fall back on, and DHCP (or the static
//! fallback in [`net`]) is the only way in.
//!
//! [W6300-EVB-Pico2]: https://docs.wiznet.io/Product/Chip/Ethernet/W6300/w6300-evb-pico2
//!
//! # Board pinout
//!
//! Pin numbers come from the `ssh-stamp-rp2350-boards` BSP crate: each PCB
//! is a `boards/*.toml`, and `build.rs` generates the `take_uart_pins!` /
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
pub use rng::getrandom_fill_bytes as rng_fill_bytes;
pub use uart::{BufferedUart, UART_BUF, UART_SIGNAL, uart_task};
