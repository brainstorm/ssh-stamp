// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

#![no_std]

mod config;
pub mod flash;
mod hash;
mod network;
mod rng;
mod timer;
mod uart;

pub use config::*;
pub use flash::{EspOtaWriter, FlashBuffer, get_flash_n_buffer, init as flash_init};
pub use hash::EspHmac;
pub use network::{EspWifi, accept_requests};
pub use rng::{EspRng, register_custom_rng};
pub use timer::EspTimer;
pub use uart::{
    BufferedUart, EspUart, EspUartPins, UART_BUF, UART_SIGNAL, uart_buffer_wait_for_initialisation,
    uart_task,
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
