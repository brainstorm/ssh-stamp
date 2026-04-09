// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! OTA traits - re-exported from hal crate
//!
//! This module exists for backward compatibility.
//! The actual trait definition is in `hal::traits::OtaActions`.

pub use hal::{HalError, OtaActions};

/// Storage error type for OTA operations
#[derive(Debug)]
pub enum StorageError {
    ReadError,
    WriteError,
    EraseError,
    InternalError,
}

/// Storage result type alias
pub type StorageResult<T> = core::result::Result<T, StorageError>;

/// Convert HalError to StorageError
impl From<HalError> for StorageError {
    fn from(_: HalError) -> Self {
        StorageError::InternalError
    }
}
