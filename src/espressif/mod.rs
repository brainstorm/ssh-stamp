// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod buffered_uart;
pub mod net;
pub mod rng;
// TODO: Specialise for Espressif, tricky since it seems to require burning eFuses?:
// https://github.com/esp-rs/esp-hal/blob/main/examples/src/bin/hmac.rs
//pub mod hash;
