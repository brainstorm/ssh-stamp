use esp_println::println;
use esp_storage::FlashStorage;
//use sequential_storage::map::store_item;
use embassy_embedded_hal::adapter::BlockingAsync;
use embedded_storage::ReadStorage;

use crate::config::SSHConfig;

pub(crate) async fn _set_value(ssh_config: SSHConfig) -> Result<(), sunset::Error> {
    let mut flash = FlashStorage::new();
    println!("Flash size = {}", flash.capacity());

    let mut flash = BlockingAsync::new(flash);

    // TODO: Define suitable ranges and buffer sizes compatible with all Espressif targets
    //store_item(&mut flash, flash_range, cache, data_buffer, key, item);

    todo!();

    Ok(())
}
