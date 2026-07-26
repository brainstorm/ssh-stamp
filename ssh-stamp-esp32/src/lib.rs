// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Angus Gratton <gus@projectgus.com>
// SPDX-FileCopyrightText: 2026 Sergio Gasquez <sergio.gasquez@gmail.com>
// SPDX-FileCopyrightText: 2026 pancake <pancake@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::mem_forget)]
#![deny(unused_imports)]
#![deny(unused_variables)]

extern crate alloc;

pub mod bench;
mod boot;
#[cfg(feature = "can")]
mod can;
pub mod flash;
mod hash;
#[cfg(feature = "i2c")]
mod i2c;
mod network;
mod platform;
mod rng;
mod timer;
mod uart;

pub use boot::start_interrupt_executor;
#[cfg(feature = "can")]
pub use can::{BufferedCan, CAN_BUF, EspCanPins, can_task};
pub use flash::{EspOtaWriter, FlashBuffer, get_flash_n_buffer, init as flash_init};
pub use hash::EspHmac;
#[cfg(feature = "i2c")]
pub use i2c::{BufferedI2c, EspI2cPins, I2C_BUF, i2c_task};
pub use network::{EspWifi, accept_requests, dhcp_server, net_up, wifi_up};
pub use platform::EspPlatform;
pub use rng::{
    EntropySource, EspRng, entropy_source_active, fill_bytes as rng_fill_bytes, init_entropy,
    register_custom_rng,
};
pub use timer::EspTimer;
pub use uart::{BufferedUart, EspUartPins, UART_BUF, UART_SIGNAL, spawn_uart, uart_task};

// These re-exports are nice to have for macro expansions, including `getrandom_backend!`,
// `init_heap!`, `start_rtos!` and `boot!, as they mean the calling crate doesn't have to
// have these as direct dependencies.
pub use esp_alloc;
pub use esp_bootloader_esp_idf;
pub use esp_hal;
pub use esp_println;
pub use esp_rtos;
pub use getrandom;
pub use log;
pub use ssh_stamp;

/// Read the device's hardware MAC address from eFuse.
#[must_use]
pub fn mac_address() -> [u8; 6] {
    let mac = esp_hal::efuse::base_mac_address();
    let bytes = mac.as_bytes();
    debug_assert_eq!(bytes.len(), 6, "eFuse MAC address must be 6 bytes");
    let mut arr = [0u8; 6];
    arr.copy_from_slice(bytes);
    arr
}
