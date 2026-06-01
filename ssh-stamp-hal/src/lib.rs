// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! # SSH-Stamp Hardware Abstraction Layer
//!
//! Platform-agnostic traits for ssh-stamp, following the embedded-hal pattern
//! of fine-grained, composable traits. Each supported platform (ESP32, BAO1X,
//! RP2350, etc.) implements these traits in separate crates.
//!
//! ## Overview
//!
//! - Peripheral traits: [`WifiHal`], [`NetworkProviderHal`], [`RngHal`],
//!   [`HashHal`], [`TimerHal`], [`OtaActions`]
//! - Configuration: [`WifiApConfigStatic`]
//! - Error handling: [`HalError`] with variants per peripheral type
//!
//! For standard peripheral traits (`Read`, `Write`, flash storage), this crate
//! defers to `embedded-io-async` and `embedded-storage-async` from the embedded-hal
//! ecosystem rather than redefining them.

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::mem_forget)]
#![deny(unused_imports)]
#![deny(unused_variables)]

pub mod config;
pub mod error;
pub mod traits;

pub use config::{UartConfig, WifiApConfigStatic};
pub use error::{FlashError, HalError, HashError, UartError, WifiError};
pub use traits::*;
