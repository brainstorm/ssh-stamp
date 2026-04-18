// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

#![no_std]

extern crate alloc;

mod config;
pub mod flash;
mod hash;
mod network;
mod rng;
mod timer;
mod uart;

pub use config::*;
pub use flash::{get_flash_n_buffer, init as flash_init, EspOtaWriter, FlashBuffer};
pub use hash::EspHmac;
pub use network::{
    accept_requests, ap_stack_disable, tcp_socket_disable, wifi_controller_disable, EspWifi,
};
pub use rng::{register_custom_rng, EspRng};
pub use timer::EspTimer;
pub use uart::{
    uart_buffer_wait_for_initialisation, uart_task, BufferedUart, EspUart, EspUartPins, UART_BUF,
    UART_SIGNAL,
};

/// Perform hardware reset.
///
/// This function never returns as it triggers a system restart.
pub fn reset() -> ! {
    #[cfg(any(
        feature = "esp32",
        feature = "esp32c2",
        feature = "esp32c3",
        feature = "esp32c6",
        feature = "esp32s2",
        feature = "esp32s3"
    ))]
    {
        esp_hal::system::software_reset()
    }

    #[cfg(not(any(
        feature = "esp32",
        feature = "esp32c2",
        feature = "esp32c3",
        feature = "esp32c6",
        feature = "esp32s2",
        feature = "esp32s3"
    )))]
    {
        loop {}
    }
}

/// Get MAC address from hardware eFuse.
#[must_use]
pub fn mac_address() -> [u8; 6] {
    #[cfg(any(
        feature = "esp32",
        feature = "esp32c2",
        feature = "esp32c3",
        feature = "esp32c6",
        feature = "esp32s2",
        feature = "esp32s3"
    ))]
    {
        esp_hal::efuse::Efuse::mac_address()
    }

    #[cfg(not(any(
        feature = "esp32",
        feature = "esp32c2",
        feature = "esp32c3",
        feature = "esp32c6",
        feature = "esp32s2",
        feature = "esp32s3"
    )))]
    {
        [0; 6]
    }
}