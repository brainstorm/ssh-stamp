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
//!
//! # Architecture
//!
//! `ssh-stamp` is firmware that turns a microcontroller into an SSH-accessible
//! serial bridge. Connect via SSH to the device, and your terminal session is
//! bridged directly to the device's UART.
//!
//! The design separates platform-agnostic logic (this crate) from
//! platform-specific implementations (port crates like `ssh-stamp-esp32`).
//! All hardware access flows through traits defined in [`ssh_stamp_hal`] or
//! through [`platform::PlatformServices`].
//!
//! ## Crate structure
//!
//! ```text
//! ssh-stamp-hal/       Trait definitions only (no implementations)
//! ssh-stamp/           Platform-agnostic application core
//! ssh-stamp-esp32/     ESP32 implementation + bootable binary
//! ssh-stamp-bao1x/     BAO1X implementation (scaffold)
//! ssh-stamp-rp2350/    RP2350 implementation (scaffold)
//! ```
//!
//! `ota` depends on `ssh-stamp-hal` for [`OtaActions`](ssh_stamp_hal::OtaActions)
//! and is in turn depended on by `ssh-stamp` for SFTP-based updates.
//!
//! ## Key modules
//!
//! - [`app`] — entry points [`prepare_ap_config`] and [`run_app`]
//! - [`handle`] — SSH event handlers (auth, channels, env vars)
//! - [`serve`] — SSH connection loop
//! - [`serial`] — UART bridge trait and bridge function
//! - [`config`] — [`SSHStampConfig`](crate::config::SSHStampConfig) struct and serialization
//! - [`store`] — Flash load/save/create
//! - [`platform`] — [`PlatformServices`](crate::platform::PlatformServices) trait (save config, reset, OTA)
//!
//! # Hacking
//!
//! ## Architectural invariants
//!
//! - **`src/` is platform-agnostic.** It must not import `esp-hal`,
//!   `esp-radio`, `esp-storage`, or any platform-specific crate. All hardware
//!   access goes through `ssh-stamp-hal` traits or `PlatformServices`.
//! - **Peripherals are owned by the state machine**, not globals. UART is an
//!   exclusive resource consumed by the serial bridge once SSH attaches.
//! - **Dependency graph is acyclic:** `ssh-stamp-hal <- ssh-stamp <-
//!   ssh-stamp-<port>`. `ssh-stamp` must not depend on any port crate.
//!
//! ## Adding a new SSH env var handler
//!
//! Edit `handle::session_env`. Add a new match arm for the variable name.
//! Follow the existing pattern: acquire the config lock, apply the change,
//! set `ctx.config_changed = true`, and call `a.succeed()`.
//!
//! ## Configuration
//!
//! On first boot (`first_login = true`), the device generates a random SSID
//! and WPA2 PSK (printed to the serial console) and accepts any SSH
//! connection. The client provisions a public key via the `SSH_STAMP_PUBKEY`
//! environment variable. Subsequent connections require that key.
//!
//! `WiFi` SSID and PSK can be changed at any time via the `SSH_STAMP_WIFI_SSID`
//! and `SSH_STAMP_WIFI_PSK` env vars. Changes are persisted to flash and the
//! device performs a software reset.
//!
//! ## Testing
//!
//! Host-side OTA TLV tests:
//! ```bash
//! cargo +stable test --package ota --target x86_64-unknown-linux-gnu
//! ```
//!
//! Manual testing requires a hardware target, a `WiFi` client, an SSH client,
//! and a serial device connected to the UART pins for bridge testing.
//!
//! [`prepare_ap_config`]: app::prepare_ap_config
//! [`run_app`]: app::run_app

#![no_std]
#![forbid(unsafe_code)]
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
