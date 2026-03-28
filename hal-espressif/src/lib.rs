// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-License-Identifier: GPL-3.0-or-later

#![no_std]

extern crate alloc;

mod config;
mod executor;
pub mod flash;
mod hash;
mod network;
mod rng;
mod timer;
mod uart;

pub use config::*;
pub use executor::EspExecutor;
pub use flash::{get_flash_n_buffer, init as flash_init, EspFlash, EspOtaWriter, FlashBuffer};
pub use hash::EspHmac;
pub use network::{
    accept_requests, ap_stack_disable, dhcp_server, init_wifi_ap, net_up, tcp_socket_disable,
    wifi_controller_disable, wifi_up, EspWifi, DEFAULT_SSID, WIFI_PASSWORD_CHARS,
};
pub use rng::EspRng;
pub use timer::EspTimer;
pub use uart::{
    uart_buffer_wait_for_initialisation, uart_task, BufferedUart, EspUart, EspUartPins, UART_BUF,
    UART_SIGNAL,
};

use embassy_executor::Spawner;
use hal::{HalError, HalPlatform, HardwareConfig};

impl HalPlatform for EspHalPlatform {
    type Uart = EspUart;
    type Wifi = EspWifi;
    type Rng = EspRng;
    type Flash = EspFlash;
    type Hash = EspHmac;
    type Timer = EspTimer;
    type Executor = EspExecutor;

    async fn init(_config: HardwareConfig, spawner: Spawner) -> Result<Self, HalError>
    where
        Self: Sized,
    {
        // Initialization is done separately in main.rs with the actual peripherals
        // This is a placeholder for future unified initialization
        Ok(Self {
            uart: EspUart::new(uart_buffer_wait_for_initialisation().await),
            wifi: EspWifi::new(),
            rng: EspRng::new(),
            flash: EspFlash,
            hash: EspHmac,
            timer: EspTimer,
            executor: EspExecutor::new(spawner),
        })
    }

    fn reset() -> ! {
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

    fn mac_address() -> [u8; 6] {
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
}

/// ESP32 platform bundle
pub struct EspHalPlatform {
    pub uart: EspUart,
    pub wifi: EspWifi,
    pub rng: EspRng,
    pub flash: EspFlash,
    pub hash: EspHmac,
    pub timer: EspTimer,
    pub executor: EspExecutor,
}
