// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! OTA traits - re-exported from ssh-stamp-hal crate
//!
//! This module exists for backward compatibility.
//! The actual trait definition is in `ssh_stamp_hal::traits::OtaActions`.

pub use ssh_stamp_hal::{HalError, OtaActions};

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

/// Convert `HalError` to `StorageError`
impl From<HalError> for StorageError {
    fn from(_: HalError) -> Self {
        StorageError::InternalError
    }
}
