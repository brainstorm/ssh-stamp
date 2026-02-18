use core::cell::RefCell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use esp_hal::rng::Rng;
use getrandom::register_custom_getrandom;
use static_cell::StaticCell;

static RNG: StaticCell<Rng> = StaticCell::new();
static RNG_MUTEX: Mutex<CriticalSectionRawMutex, RefCell<Option<&'static mut Rng>>> =
    Mutex::new(RefCell::new(None));

pub fn register_custom_rng(rng: Rng) {
    let rng = RNG.init(rng);
    RNG_MUTEX.lock(|t| t.borrow_mut().replace(rng));
    register_custom_getrandom!(esp_getrandom_custom_func);
}

// esp-hal specific variation of getrandom custom function as seen in:
// https://github.com/rust-random/getrandom/issues/340
pub fn esp_getrandom_custom_func(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    RNG_MUTEX.lock(|t| {
        let mut rng = t.borrow_mut();
        let rng = rng
            .as_mut()
            .expect("register_custom_rng should have set this");
        rng.read(buf);
    });
    Ok(())
}
