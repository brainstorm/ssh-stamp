// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 gabriel.ku <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

#![no_std]
#![no_main]
// #![forbid(unsafe_code)]
#[deny(clippy::mem_forget)] // avoids any UB, forces use of Drop impl instead
#[deny(unused_imports)] // avoid accidentally leaving in debug imports
#[deny(unused_variables)] // avoid accidentally leaving in debug variables
pub mod config;
pub mod errors;
pub mod espressif;
pub mod handle;
pub mod serial;
pub mod serve;
pub mod settings;
pub mod store;
