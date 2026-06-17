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
//! of fine-grained, composable traits. Each supported platform implements
//! these traits in a separate port crate (e.g. `ssh-stamp-esp32`).
//!
//! ## Overview
//!
//! - Peripheral traits: [`WifiHal`], [`NetworkProviderHal`], [`RngHal`],
//!   [`HashHal`], [`TimerHal`], [`OtaActions`]
//! - Configuration: [`WifiApConfigStatic`]
//! - Error handling: [`HalError`] with variants per peripheral type
//!
//! For standard peripheral traits (`Read`, `Write`, flash storage), this crate
//! defers to `embedded-io-async` and `embedded-storage-async` from the
//! embedded-hal ecosystem rather than redefining them.
//!
//! ## HAL trait map
//!
//! | Trait                    | Required?    | ESP32 impl      |
//! |-------------------------|--------------|------------------|
//! | [`NetworkProviderHal`]   | always       | `EspWifi`         |
//! | [`WifiHal`]              | `WiFi` ports | `EspWifi`         |
//! | `BufferedSerial`          | always       | `BufferedUart`    |
//! | [`OtaActions`]           | sftp-ota     | `EspOtaWriter`      |
//! | `PlatformServices`        | always       | `EspPlatform`      |
//!
//! [`WifiHal`] is required only for WiFi-based ports. Ethernet ports would
//! implement [`NetworkProviderHal`] directly.
//!
//! ## Adding a new port
//!
//! To port ssh-stamp to a new microcontroller family:
//!
//! 1. Create lib and bin for your platform: `ssh-stamp/ssh-stamp-yourplatform/src/lib.rs` and `ssh-stamp/ssh-stamp-yourplatform/src/bin/ssh-stamp-yourplatform.rs`.
//! 2. Implement the needed traits from `ssh-stamp-hal/src/traits/`. At a
//!    minimum: a [`NetworkProviderHal`] (or [`WifiHal`]), [`OtaActions`], and
//!    a UART type implementing the `BufferedSerial` trait from the `ssh-stamp`
//!    crate.
//! 3. Implement the `PlatformServices` trait from `ssh-stamp::platform` for
//!    the platform.
//! 4. In the binary, mirror the ESP32 boot flow: bring up peripherals, load
//!    config via `ssh-stamp::store::load_or_create`, spawn the UART task,
//!    bring up the network, call `ssh-stamp::app::run_app`.
//! 5. Add a `cargo build-yourplatform` alias in `.cargo/config.toml`.
//!
//! No changes are needed in `ssh-stamp` or `ssh-stamp-hal`.

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
