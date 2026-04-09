// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! HAL error types.
//!
//! This module defines error enums for all HAL operations. Each error type
//! corresponds to a specific peripheral or operation category.

use core::fmt;

/// Unified HAL error type.
///
/// Aggregates all peripheral-specific errors into a single enum for
/// ergonomic error handling across the HAL.
#[derive(Debug)]
pub enum HalError {
    /// Configuration error (invalid settings).
    Config,
    /// UART peripheral error.
    Uart(UartError),
    /// WiFi peripheral error.
    Wifi(WifiError),
    /// Flash storage error.
    Flash(FlashError),
    /// Random number generator error.
    Rng,
    /// Hash/HMAC computation error.
    Hash(HashError),
    /// Timer error.
    Timer,
    /// Async executor error.
    Executor,
}

/// UART-specific errors.
#[derive(Debug)]
pub enum UartError {
    /// Invalid UART configuration.
    Config,
    /// Receive buffer overflow (data lost).
    BufferOverflow,
    /// Read operation failed.
    Read,
    /// Write operation failed.
    Write,
}

/// WiFi-specific errors.
#[derive(Debug)]
pub enum WifiError {
    /// WiFi hardware initialization failed.
    Initialization,
    /// Failed to create socket.
    SocketCreate,
    /// Failed to accept connection.
    SocketAccept,
    /// Socket read failed.
    SocketRead,
    /// Socket write failed.
    SocketWrite,
    /// Socket close failed.
    SocketClose,
    /// DHCP client error.
    Dhcpc,
}

/// Flash storage errors.
#[derive(Debug)]
pub enum FlashError {
    /// Read operation failed.
    Read,
    /// Write operation failed.
    Write,
    /// Erase operation failed.
    Erase,
    /// Requested partition not found.
    PartitionNotFound,
    /// OTA partition validation failed.
    ValidationFailed,
    /// Failed to load configuration from flash.
    ConfigLoad,
    /// Failed to save configuration to flash.
    ConfigSave,
    /// Internal flash controller error.
    InternalError,
}

/// Hash/HMAC computation errors.
#[derive(Debug)]
pub enum HashError {
    /// Invalid hash/HMAC configuration.
    Config,
    /// Hash computation failed.
    Compute,
}

impl fmt::Display for HalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HalError::Config => write!(f, "configuration error"),
            HalError::Uart(e) => write!(f, "UART error: {:?}", e),
            HalError::Wifi(e) => write!(f, "WiFi error: {:?}", e),
            HalError::Flash(e) => write!(f, "Flash error: {:?}", e),
            HalError::Rng => write!(f, "RNG error"),
            HalError::Hash(e) => write!(f, "Hash error: {:?}", e),
            HalError::Timer => write!(f, "Timer error"),
            HalError::Executor => write!(f, "Executor error"),
        }
    }
}
