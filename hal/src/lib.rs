// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

#![no_std]

pub mod config;
pub mod error;
pub mod traits;

pub use config::*;
pub use error::{FlashError, HalError, HashError, UartError, WifiError};
pub use traits::*;

use core::future::Future;
use embassy_executor::Spawner;

/// Platform abstraction bundling all HAL peripherals
///
/// Each platform implementation (ESP32, Nordic nRF, etc.) implements this trait
/// to provide access to all hardware peripherals needed by the firmware.
pub trait HalPlatform {
    type Uart: UartHal;
    type Wifi: WifiHal;
    type Rng: RngHal;
    type Flash: FlashHal;
    type Hash: HashHal;
    type Timer: TimerHal;
    type Executor: ExecutorHal;

    /// Initialize all peripherals with given configuration
    fn init(
        config: HardwareConfig,
        spawner: Spawner,
    ) -> impl Future<Output = Result<Self, HalError>>
    where
        Self: Sized;

    /// Perform hardware reset
    fn reset() -> !;

    /// Get MAC address from hardware (eFuse or similar)
    fn mac_address() -> [u8; 6];
}
