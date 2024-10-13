use core::fmt::Display;

#[derive(Debug)]
pub enum EspSshError {
    ClientError(zssh::Error<embassy_net::tcp::Error>),
    CertificateError(),
}

impl From<zssh::Error<embassy_net::tcp::Error>> for EspSshError {
    fn from(inner: zssh::Error<embassy_net::tcp::Error>) -> Self {
        EspSshError::ClientError(inner)
    }
}

impl From<ed25519_dalek::ed25519::Error> for EspSshError {
    fn from(_: ed25519_dalek::ed25519::Error) -> Self {
        // The inner error type here deliberately doesn't contain any context
        EspSshError::CertificateError()
    }
}

// NOTE: This is a pretty bare-bones implementation of Display, but Debug is
// derived and should contain all available detail. It may be better to remove
// this and nudge anyone printing the error to Debug, as the designated audience
// is probably Debug-level...
impl Display for EspSshError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (variant, inner) = match self {
            EspSshError::ClientError(e) => (
                "SSH Client Error",
                Some(match e {
                    zssh::Error::UnexpectedEof => "Unexpected EOF",
                    zssh::Error::IO(_) => "I/O Error",
                    zssh::Error::Protocol(_protocol_error) => "SSH Protocol error",
                    zssh::Error::ServerDisconnect(_disconnect_reason) => "Server Disconnect",
                    zssh::Error::ClientDisconnect(_disconnect_reason) => "Client Disconnect",
                }),
            ),
            EspSshError::CertificateError() => ("Certificate Error", None),
        };
        write!(
            f,
            "Fatal Error: {}{}{}",
            variant,
            if inner.is_some() { ": " } else { "" },
            inner.unwrap_or("")
        )
    }
}
