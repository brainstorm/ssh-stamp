// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! USB host serial backend for the ESP32-S2/S3.
//!
//! Runs the USB OTG port in host mode and bridges an attached USB CDC-ACM
//! device (e.g. a board's `ttyACM` console) through the same
//! [`BufferedUart`] pipes the UART backend fills, so the SSH side is
//! unchanged. Vendor-class USB-UART adapters are not supported yet.

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_time::Timer;
use embassy_usb_driver::host::{HostError, PipeError, UsbHostAllocator, UsbPipe, pipe};
use embassy_usb_driver::{EndpointInfo, EndpointType};
use embassy_usb_host::class::cdc_acm::find_cdc_acm;
use embassy_usb_host::control::ControlPipeExt;
use embassy_usb_host::handler::EnumerationInfo;
use embassy_usb_host::{BusRoute, BusState};
use esp_hal::usb::otg::{Usb, embassy_usb_host::Driver};
use log::{info, warn};
use ssh_stamp_hal::{Parity, UartParams};

use crate::uart::{BufferedUart, UART_BUF, UART_SIGNAL};

// CDC PSTN class requests.
const SET_LINE_CODING: u8 = 0x20;
const SET_CONTROL_LINE_STATE: u8 = 0x22;
/// DTR and RTS asserted: many devices stay silent until the port is "open".
const DTR_RTS: u16 = 0b11;

/// Largest full-speed bulk packet.
const PACKET_SZ: usize = 64;
const CONFIG_BUF_SZ: usize = 512;

/// Embassy task that runs the USB OTG port as a host and bridges each
/// attached CDC-ACM device through `uart_buf`. Like
/// [`uart_task`](crate::uart_task), it waits on [`UART_SIGNAL`] first.
#[embassy_executor::task]
pub async fn usb_host_task(uart_buf: &'static BufferedUart, usb: Usb<'static>, params: UartParams) {
    static BUS: BusState = BusState::new();

    UART_SIGNAL.wait().await;

    let (mut bus, handle) = embassy_usb_host::bus(Driver::new(usb), &BUS);
    let mut config = [0u8; CONFIG_BUF_SZ];
    // Must outlive every IN pipe: the driver keeps a pointer to it until
    // the channel is reused, even after a transfer is dropped.
    let mut packet = [0u8; PACKET_SZ];

    loop {
        let speed = bus.wait_for_connection().await;
        let (dev, len) = match handle.enumerate(BusRoute::Direct(speed), &mut config).await {
            Ok(enumerated) => enumerated,
            Err(e) => {
                warn!("USB enumeration failed: {e}");
                continue;
            }
        };

        // Watching the port is what fails in-flight transfers on unplug.
        let bridge = bridge(&handle, &dev, &config[..len], params, uart_buf, &mut packet);
        match select(bus.wait_for_device_event(), bridge).await {
            Either::First(event) => info!("USB device event: {event:?}"),
            Either::Second(Ok(())) => info!("USB device disconnected"),
            Either::Second(Err(e)) => warn!("USB device not bridged: {e:?}"),
        }
        handle.free_address(dev.device_address);
    }
}

/// Sets up the CDC-ACM interface of `dev` and pumps bytes between it and
/// `uart_buf` until the device goes away.
///
/// Transfers are never cancelled while the device is attached: the driver
/// would lose the in-flight packet and the data toggle.
async fn bridge<'d>(
    alloc: &impl UsbHostAllocator<'d>,
    dev: &EnumerationInfo,
    config: &[u8],
    params: UartParams,
    uart_buf: &BufferedUart,
    packet: &mut [u8; PACKET_SZ],
) -> Result<(), HostError> {
    let acm = find_cdc_acm(config).ok_or(HostError::Other("no CDC-ACM interface"))?;
    let addr = dev.device_address;
    let ep0_mps = u16::from(dev.device_desc.max_packet_size0);

    // Best effort: some devices stall requests they do not implement.
    let mut ctrl = alloc.alloc_pipe::<pipe::Control, pipe::InOut>(
        addr,
        &endpoint(0, EndpointType::Control, ep0_mps),
        dev.split(),
    )?;
    let iface = u16::from(acm.comm_interface);
    if let Err(e) = ctrl
        .class_request_out(SET_LINE_CODING, 0, iface, &line_coding(params))
        .await
    {
        warn!("CDC-ACM SET_LINE_CODING failed: {e:?}");
    }
    if let Err(e) = ctrl
        .class_request_out(SET_CONTROL_LINE_STATE, DTR_RTS, iface, &[])
        .await
    {
        warn!("CDC-ACM SET_CONTROL_LINE_STATE failed: {e:?}");
    }
    drop(ctrl);

    let mut rx = alloc.alloc_pipe::<pipe::Bulk, pipe::In>(
        addr,
        &endpoint(acm.bulk_in_ep, EndpointType::Bulk, acm.bulk_in_mps),
        dev.split(),
    )?;
    let mut tx = alloc.alloc_pipe::<pipe::Bulk, pipe::Out>(
        addr,
        &endpoint(acm.bulk_out_ep, EndpointType::Bulk, acm.bulk_out_mps),
        dev.split(),
    )?;
    info!("USB CDC-ACM device bridged");

    // One packet per transfer, so a full packet is not held back waiting
    // for the rest of a larger transfer.
    let rx_len = usize::from(acm.bulk_in_mps).min(PACKET_SZ);
    let inward = async {
        loop {
            match rx.request_in(&mut packet[..rx_len]).await {
                Ok(n) => uart_buf.push_inward(&packet[..n]),
                Err(PipeError::Disconnected) => return,
                Err(e) => back_off("IN", e).await,
            }
        }
    };
    let outward = async {
        let mut chunk = [0u8; PACKET_SZ];
        loop {
            let n = uart_buf.pull_outward(&mut chunk).await;
            // Resending is safe: the data toggle makes the device drop duplicates.
            while let Err(e) = tx.request_out(&chunk[..n], false).await {
                if e == PipeError::Disconnected {
                    return;
                }
                back_off("OUT", e).await;
            }
        }
    };
    select(inward, outward).await;
    Ok(())
}

/// Logs a transfer error and pauses, so a persistent fault does not spin
/// the executor.
async fn back_off(dir: &str, e: PipeError) {
    warn!("USB {dir} transfer failed: {e:?}");
    Timer::after_millis(100).await;
}

/// `addr` is the descriptor's `bEndpointAddress`, direction bit included.
fn endpoint(addr: u8, ep_type: EndpointType, max_packet_size: u16) -> EndpointInfo {
    EndpointInfo {
        addr: addr.into(),
        ep_type,
        max_packet_size,
        interval_ms: 0,
    }
}

/// `SET_LINE_CODING` payload (CDC PSTN 1.2, table 17).
fn line_coding(params: UartParams) -> [u8; 7] {
    let [b0, b1, b2, b3] = params.baud.to_le_bytes();
    // bCharFormat: 0 is one stop bit, 2 is two.
    let stop_bits = if params.stop_bits == 2 { 2 } else { 0 };
    let parity = match params.parity {
        Parity::None => 0,
        Parity::Odd => 1,
        Parity::Even => 2,
    };
    [b0, b1, b2, b3, stop_bits, parity, params.data_bits]
}

/// USB host counterpart of [`spawn_uart`](crate::spawn_uart): creates the
/// [`BufferedUart`] singleton and spawns [`usb_host_task`] on `spawner`.
///
/// Uses the thread-mode executor: the OTG driver retries NAKs and waits for
/// channel halts in software, which at the interrupt executor's priority
/// would starve thread mode and block the USB interrupt. The device holds
/// its data until polled, so nothing is lost.
///
/// # Panics
///
/// Panics if called more than once per boot, or alongside `spawn_uart`:
/// both claim the same [`BufferedUart`] singleton.
pub fn spawn_usb_host(
    spawner: Spawner,
    usb: Usb<'static>,
    params: UartParams,
) -> &'static BufferedUart {
    let uart_buf = UART_BUF.init_with(BufferedUart::new);
    spawner.spawn(usb_host_task(uart_buf, usb, params).expect("usb_host_task spawn failed"));
    uart_buf
}
