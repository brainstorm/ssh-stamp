// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The HIL tests for ssh-stamp.
//!
//! Each file under `tests/` is an [embedded-test] binary which uses probe-rs
//! to flash the image and execute the test. These should be run through xtask:
//!
//! ```sh
//! cargo xtask esp32c6-devkitc test
//! ```
//!
//! [embedded-test]: https://github.com/probe-rs/embedded-test

#![no_std]

// The application descriptor the esp-idf bootloader expects in the app image.
esp_bootloader_esp_idf::esp_app_desc!();

// The `getrandom` custom backend.
ssh_stamp_esp32::getrandom_backend!();
