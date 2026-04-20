// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 gabriel.ku <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Peripheral trait definitions.
//!
//! This module re-exports all HAL trait definitions. Each trait provides
//! an abstract interface for a specific hardware peripheral.

mod flash;
mod hash;
mod network;
mod rng;
mod timer;
mod uart;

pub use flash::OtaActions;
pub use hash::HashHal;
pub use network::WifiHal;
pub use rng::RngHal;
pub use timer::TimerHal;
pub use uart::UartHal;
