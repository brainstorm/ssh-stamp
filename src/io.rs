use core::fmt::Debug;

use embassy_net::tcp::TcpSocket;
use embedded_io_async::{ErrorType, Read, Write};

// Newtype because TcpSocket derives Debug marker trait
pub struct DebuggableTcpSocket<'a>(TcpSocket<'a>);
pub struct AsyncTcpStream<'a>(pub(crate) DebuggableTcpSocket<'a>);

impl<'a> Debug for DebuggableTcpSocket<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Implement the Debug trait for TcpSocket<'a>
        // You can customize the formatting logic here if needed
        write!(f, "TcpSocket")
    }
}

impl<'a> ErrorType for AsyncTcpStream<'a> {
    type Error = embassy_net::tcp::Error;
}
impl<'a> Read for AsyncTcpStream<'a> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.0.read(buf).await.map_err(|e| e.into())
    }
}

impl<'a> Write for AsyncTcpStream<'a> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.0.write(buf).await.map_err(|e| e.into())
    }
}