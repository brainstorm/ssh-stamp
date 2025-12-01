#![no_std]
#![no_main]
// #![forbid(unsafe_code)]

// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later
#[deny(clippy::mem_forget)] // avoids any UB, forces use of Drop impl instead
pub mod config;
pub mod pins;
pub mod errors;
pub mod espressif;
pub mod keys;
pub mod serial;
pub mod serve;
pub mod settings;
pub mod storage;
