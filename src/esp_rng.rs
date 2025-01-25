use esp_hal::peripherals::Peripherals;
use esp_hal::rng::Trng;

pub fn esp_random(peripherals: &mut Peripherals) -> Trng {
    let adc1 = &mut peripherals.ADC1;
    Trng::new(&mut peripherals.RNG, adc1)   
}

// esp-hal specific variation of getrandom custom function as seen in:
// https://github.com/rust-random/getrandom/issues/340
pub fn esp_getrandom_custom_func(buf: &mut [u8]) -> Result<(), getrandom::Error>{
    let mut peripherals = esp_hal::init(esp_hal::Config::default());
    esp_random(&mut peripherals).read(buf);
    Ok(())
}