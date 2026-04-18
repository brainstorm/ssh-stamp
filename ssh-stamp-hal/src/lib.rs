// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! # SSH-Stamp Hardware Abstraction Layer
//!
//! Platform-agnostic traits for ssh-stamp, following the embedded-hal pattern
//! of fine-grained, composable traits. Each supported platform (ESP32, nRF52, etc.)
//! implements these traits in separate crates (e.g., `ssh-stamp-esp32`).
//!
//! ## Overview
//!
//! - Peripheral traits: [`WifiHal`], [`RngHal`], [`HashHal`], [`TimerHal`], [`OtaActions`]
//! - Configuration: [`HardwareConfig`], [`UartConfig`], [`WifiApConfigStatic`]
//! - Error handling: [`HalError`] with variants per peripheral type
//!
//! For standard peripheral traits (`Read`, `Write`, flash storage), this crate
//! defers to `embedded-io-async` and `embedded-storage-async` from the embedded-hal
//! ecosystem rather than redefining them.

#![no_std]

pub mod config;
pub mod error;
pub mod traits;

pub use config::*;
pub use error::{FlashError, HalError, HashError, UartError, WifiError};
pub use traits::*;