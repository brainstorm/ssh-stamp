use esp_hal::hmac::Hmac;
use hmac::Hmac;
use sha2::Sha256;

pub trait EspressifHmac {
    fn new_from_slice(key: &[u8]) -> Result<Self, ()>
    where
        Self: Sized;
    fn update(&mut self, data: &[u8]);
    fn finalize(self) -> [u8; 32];
}

impl EspressifHmac for Hmac<Sha256> {
    fn new_from_slice(key: &[u8]) -> Result<Self, ()> {
        Hmac::new_from_slice(key).map_err(|_| ())
    }

    fn update(&mut self, data: &[u8]) {
        self.update(data);
    }

    fn finalize(self) -> [u8; 32] {
        let result = self.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&result.into_bytes());
        arr
    }
}