// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! # Hardware Abstraction Layer (HAL)
//!
//! This crate provides platform-agnostic hardware abstraction traits for embedded systems.
//! Each supported platform (ESP32, nRF52, etc.) implements these traits in separate crates.
//!
//! ## Overview
//!
//! The HAL is organized around several key concepts:
//!
//! - [`HalPlatform`]: Top-level trait bundling all peripherals together
//! - Peripheral traits: [`UartHal`], [`WifiHal`], [`RngHal`], [`FlashHal`], [`HashHal`], [`TimerHal`], [`ExecutorHal`]
//! - Configuration: [`HardwareConfig`], [`UartConfig`], [`WifiApConfigStatic`], [`EthernetConfig`]
//! - Error handling: [`HalError`] with variants for each peripheral type
//!
//! ## Usage
//!
//! Platform implementations (e.g., `hal-espressif`) implement these traits, and applications
//! use the trait bounds to write platform-agnostic code.
//!
//! ```ignore
//! use hal::{HalPlatform, HardwareConfig};
//!
//! async fn init_peripherals<P: HalPlatform>(config: HardwareConfig) -> Result<P, HalError> {
//!     P::init(config, spawner).await
//! }
//! ```

#![no_std]

pub mod config;
pub mod error;
pub mod traits;

pub use config::*;
pub use error::{FlashError, HalError, HashError, UartError, WifiError};
pub use traits::*;

use core::future::Future;
use embassy_executor::Spawner;

/// Platform abstraction bundling all HAL peripherals.
///
/// Each platform implementation (ESP32, Nordic nRF, etc.) implements this trait
/// to provide access to all hardware peripherals needed by the firmware.
///
/// # Example Implementation
///
/// ```ignore
/// struct EspPlatform {
///     uart: EspUart,
///     wifi: EspWifi,
///     // ...
/// }
///
/// impl HalPlatform for EspPlatform {
///     type Uart = EspUart;
///     // ...
///
///     async fn init(config: HardwareConfig, spawner: Spawner) -> Result<Self, HalError> {
///         // Initialize hardware...
///     }
/// }
/// ```
pub trait HalPlatform {
    /// UART peripheral implementation.
    type Uart: UartHal;

    /// WiFi peripheral implementation.
    type Wifi: WifiHal;

    /// Random number generator implementation.
    type Rng: RngHal;

    /// Flash storage implementation.
    type Flash: FlashHal;

    /// Hash/HMAC implementation.
    type Hash: HashHal;

    /// Timer implementation.
    type Timer: TimerHal;

    /// Async executor implementation.
    type Executor: ExecutorHal;

    /// Initialize all peripherals with given configuration.
    ///
    /// This method should be called once at startup to configure and instantiate
    /// all hardware peripherals.
    ///
    /// # Arguments
    ///
    /// * `config` - Hardware configuration including pin assignments and settings
    /// * `spawner` - Embassy executor spawner for spawning async tasks
    ///
    /// # Returns
    ///
    /// Returns `Ok(Self)` on successful initialization, or `Err(HalError)` on failure.
    fn init(
        config: HardwareConfig,
        spawner: Spawner,
    ) -> impl Future<Output = Result<Self, HalError>>
    where
        Self: Sized;

    /// Perform hardware reset.
    ///
    /// This function never returns as it triggers a system restart.
    fn reset() -> !;

    /// Get MAC address from hardware (eFuse or similar).
    ///
    /// Returns the factory-programmed MAC address for the device.
    fn mac_address() -> [u8; 6];
}
