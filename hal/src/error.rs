// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

use core::fmt;

#[derive(Debug)]
pub enum HalError {
    Config,
    Uart(UartError),
    Wifi(WifiError),
    Flash(FlashError),
    Rng,
    Hash(HashError),
    Timer,
    Executor,
}

#[derive(Debug)]
pub enum UartError {
    Config,
    BufferOverflow,
    Read,
    Write,
}

#[derive(Debug)]
pub enum WifiError {
    Initialization,
    SocketCreate,
    SocketAccept,
    SocketRead,
    SocketWrite,
    SocketClose,
    Dhcpc,
}

#[derive(Debug)]
pub enum FlashError {
    Read,
    Write,
    Erase,
    PartitionNotFound,
    ValidationFailed,
    ConfigLoad,
    ConfigSave,
    InternalError,
}

#[derive(Debug)]
pub enum HashError {
    Config,
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
