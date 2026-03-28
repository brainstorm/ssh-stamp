// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Buffered UART support with app-specific configuration
//!
//! Re-exports from hal-espressif and provides app-specific task wrappers.

// Re-export core types from HAL
pub use hal_espressif::{BufferedUart, UART_BUF, UART_SIGNAL};

use crate::config::SSHStampConfig;
use esp_hal::gpio::AnyPin;
use esp_hal::peripherals::UART1;
use esp_hal::system::software_reset;
use esp_hal::uart::{Config, RxConfig, Uart};
use hal_espressif::EspUartPins;
use sunset_async::SunsetMutex;

/// UART pins wrapper for app compatibility
pub struct UartPins<'a> {
    pub rx: AnyPin<'a>,
    pub tx: AnyPin<'a>,
}

impl<'a> From<UartPins<'a>> for EspUartPins<'a> {
    fn from(pins: UartPins<'a>) -> Self {
        EspUartPins {
            rx: pins.rx,
            tx: pins.tx,
        }
    }
}

/// UART task for Embassy executor  
#[embassy_executor::task]
pub async fn uart_task(
    uart_buf: &'static BufferedUart,
    uart1: UART1<'static>,
    config: &'static SunsetMutex<SSHStampConfig>,
    pins: UartPins<'static>,
) {
    // Config is reserved for future use in parameter reconfiguration
    let _ = config;

    // Wait until SSH shell is ready
    UART_SIGNAL.wait().await;

    // Hardware UART setup - pins are selected at compile time
    let uart_config = Config::default().with_rx(
        RxConfig::default()
            .with_fifo_full_threshold(16)
            .with_timeout(1),
    );

    // Silent failure to avoid println! in interrupt context
    let Ok(uart) = Uart::new(uart1, uart_config) else {
        error!("Uart config error. Resetting.");
        software_reset();
        return;
    };
    let hal_pins: EspUartPins = pins.into();
    let uart = uart.with_rx(hal_pins.rx).with_tx(hal_pins.tx).into_async();

    // Run the main buffered TX/RX loop
    uart_buf.run(uart).await;
}

/// Wait for UART buffer initialization
pub async fn uart_buffer_wait_for_initialisation() -> &'static BufferedUart {
    hal_espressif::uart_buffer_wait_for_initialisation().await
}

pub fn uart_buffer_disable() {
    debug!("UART buffer disabled: WIP");
}

pub fn uart_disable() {
    debug!("UART disabled: WIP");
}

use log::{debug, error};