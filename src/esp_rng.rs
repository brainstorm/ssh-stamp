use esp_hal::peripherals::Peripherals;
use esp_hal::rng::Trng;

pub fn esp_random(peripherals: &mut Peripherals) -> Trng {
    let adc1 = &mut peripherals.ADC1;
    Trng::new(&mut peripherals.RNG, adc1)   
}