use core::fmt::Debug;

use embassy_net::tcp::TcpSocket;
use embedded_io_async::{ErrorType, Read, Write};

// Newtype because TcpSocket does not derive Debug
pub struct DebuggableTcpSocket<'a>(pub TcpSocket<'a>);
pub struct AsyncTcpStream<'a>(pub TcpSocket<'a>);

impl Debug for DebuggableTcpSocket<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "TcpSocket> ")
    }
}

impl ErrorType for AsyncTcpStream<'_> {
    type Error = embassy_net::tcp::Error;
}
impl Read for AsyncTcpStream<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}

impl Write for AsyncTcpStream<'_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }
}
