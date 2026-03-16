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
