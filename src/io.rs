use core::fmt::Debug;

use embassy_net::tcp::TcpSocket;
use embedded_io_async::{ErrorType, Read, Write};

// Newtype because TcpSocket does not derive Debug
pub struct DebuggableTcpSocket<'a>(pub TcpSocket<'a>);
pub struct AsyncTcpStream<'a>(pub TcpSocket<'a>);

impl<'a> Debug for DebuggableTcpSocket<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "TcpSocket> ")
    }
}

impl<'a> ErrorType for AsyncTcpStream<'a> {
    type Error = embassy_net::tcp::Error;
}
impl<'a> Read for AsyncTcpStream<'a> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}

impl<'a> Write for AsyncTcpStream<'a> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }
}