// use esp_hal::uart::{Config, Uart};
// use esp_hal::Async;
// use esp_hal::peripherals::Peripherals;

// pub(crate) fn init_uart(peripherals: UART1) -> Uart<'static, Async> {
//     let config = Config::default().with_rx_timeout(1);

//     Uart::new(peripherals.UART1, config)
//         .unwrap()
//         .with_rx(peripherals.GPIO11)
//         .with_tx(peripherals.GPIO10)
//         .into_async()
// }
