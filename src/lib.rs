// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Platform-agnostic core of `ssh-stamp`.
//!
//! Hosts the SSH state machine, configuration handling, and the
//! [`platform::PlatformServices`] / [`serial::BufferedSerial`] traits that a
//! per-MCU adapter crate (e.g. `ssh-stamp-esp32`) implements.

#![no_std]
// #![forbid(unsafe_code)]
#![deny(clippy::mem_forget)]
#![deny(unused_imports)]
#![deny(unused_variables)]

extern crate alloc;

pub mod app;
pub mod config;
pub mod errors;
pub mod handle;
pub mod platform;
pub mod serial;
pub mod serve;
pub mod settings;
pub mod store;
