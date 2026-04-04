#![no_std]
#![no_main]
// #![forbid(unsafe_code)]

// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later
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
