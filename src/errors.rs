use snafu::Snafu;
use core::result;

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Invalid PIN provided"))]
    InvalidPin,
    #[snafu(display("Flash storage error"))]
    FlashStorageError,
}