#![no_std]
#![no_main]

use esp_backtrace as _;
use core::marker::Sized;

use embassy_executor::Spawner;
use esp_ssh_rs::serve::start;

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    //esp_println::logger::init_logger_from_env();

    let _ = start(spawner).await;
    loop {}
}