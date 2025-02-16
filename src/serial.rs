use embassy_futures::select::select;
use embedded_io_async::{Read, Write};
use esp_println::println;
use heapless::Vec;

use crate::takepipe::{self, TakePipe};

/// Forwards an incoming SSH connection to a local serial port, either uart or USB
pub(crate) async fn serial<R, W>(
    chanr: &mut R,
    chanw: &mut W,
    serial_pipe: &'static TakePipe<'static>,
) -> Result<(), sunset::Error>
where
    R: Read<Error = embassy_net::tcp::Error>,
    W: Write<Error = embassy_net::tcp::Error>,
{
    println!("start serial");
    let (mut rx, mut tx) = serial_pipe.take().await;
    let r = async {
        // TODO: could have a single buffer to translate in-place.
        const DOUBLE: usize = 2 * takepipe::READ_SIZE;
        loop {
            let mut b = [0u8; takepipe::READ_SIZE];
            let n = rx.read(&mut b).await?;
            let b = &mut b[..n];
            let mut btrans = Vec::<u8, DOUBLE>::new();
            for c in b {
                if *c == b'\n' {
                    // OK unwrap: btrans.len() = 2*b.len()
                    btrans.push(b'\r').unwrap();
                }
                btrans.push(*c).unwrap();
            }
            // FIXME: Extend sunset::Error to implement embassy_net::tcp::Error
            //chanw.write_all(&btrans).await?;
            chanw.write_all(&btrans).await.unwrap();
        }
        #[allow(unreachable_code)]
        Ok::<(), sunset::Error>(())
    };
    let w = async {
        let mut b = [0u8; 64];
        loop {
            // FIXME: Extend sunset::Error to implement embassy_net::tcp::Error
            //let n = chanr.read(&mut b).await?;
            let n = chanr.read(&mut b).await.unwrap();
            if n == 0 {
                return Err(sunset::Error::ChannelEOF);
            }
            let b = &mut b[..n];
            for c in b.iter_mut() {
                // input translate CR to LF
                if *c == b'\r' {
                    *c = b'\n';
                }
            }
            tx.write_all(b).await?;
        }
        #[allow(unreachable_code)]
        Ok::<(), sunset::Error>(())
    };

    select(r, w).await;
    println!("serial task completed");
    Ok(())
}