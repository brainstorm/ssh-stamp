// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Peripheral trait definitions.
//!
//! This module re-exports all HAL trait definitions. Each trait provides
//! an abstract interface for a specific hardware peripheral.

mod executor;
mod flash;
mod hash;
mod network;
mod rng;
mod timer;
mod uart;

pub use executor::ExecutorHal;
pub use flash::{FlashHal, OtaActions};
pub use hash::HashHal;
pub use network::{EthernetHal, WifiHal};
pub use rng::RngHal;
pub use timer::TimerHal;
pub use uart::UartHal;