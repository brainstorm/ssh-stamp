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
//! The `getrandom` backend the tests need comes from `ssh-stamp-esp32-entropy`,
//! which is linked through `ssh-stamp-esp32`.
//!
//! [embedded-test]: https://github.com/probe-rs/embedded-test

#![no_std]
