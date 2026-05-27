// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Application-level error types.

use core::result;
use snafu::Snafu;

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Invalid PIN provided"))]
    InvalidPin,
    #[snafu(display("Flash storage error"))]
    FlashStorageError,
    BadKey,
    OpenSSHParseError,
}

impl From<ssh_key::Error> for Error {
    fn from(_e: ssh_key::Error) -> Error {
        Error::OpenSSHParseError
    }
}
