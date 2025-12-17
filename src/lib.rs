#![no_std]
#![no_main]
#![forbid(unsafe_code)]

// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod espressif;
pub mod keys;
pub mod serial;
pub mod serve;
pub mod settings;

mod ota;