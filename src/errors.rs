use thiserror::Error;

#[derive(Error, Debug)]
pub enum EspSshError {
    #[error("oops")]
    ConnectionReset(#[from] zssh::Error<embassy_net::tcp::Error>),
}