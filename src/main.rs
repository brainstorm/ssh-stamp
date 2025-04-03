#![no_std]
#![no_main]

use core::marker::Sized;
use esp_alloc as _;
use esp_backtrace as _;
use esp_println::println;

use embassy_executor::Spawner;
use ssh_stamp::serve::start;

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    esp_alloc::heap_allocator!(size: 72 * 1024);
    esp_println::logger::init_logger_from_env();

    let res = start(spawner).await;
    if let Err(e) = res {
        println!("Giving up: {:?}", e);
    }
    todo!(); // try again somehow
}
