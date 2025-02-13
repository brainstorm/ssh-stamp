#![no_std]
#![no_main]

use esp_alloc as _;
use esp_backtrace as _;
use esp_println::println;
use core::marker::Sized;

use embassy_executor::Spawner;
use esp_ssh_rs::serve::start;

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    esp_alloc::heap_allocator!(72 * 1024);
    esp_println::logger::init_logger_from_env();

    let res = start(spawner).await;
    if let Err(e) = res {
        println!("Giving up: {:?}", e);
    }
    todo!(); // try again somehow
}
