use core::net::Ipv4Addr;
use embassy_net::{Ipv4Cidr, StaticConfigV4};
use esp_hal::gpio::AnyPin;
use esp_hal::peripherals;
use heapless::{String, Vec};

use esp_println::println;

use bcrypt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use sunset::error::TrapBug;
use sunset::{KeyType, Result};
use sunset::{
    packets::Ed25519PubKey,
    sshwire::{SSHDecode, SSHEncode, SSHSink, SSHSource, WireError, WireResult},
    SignKey,
};
use embassy_sync::channel::Channel;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use crate::errors;
use crate::settings::{DEFAULT_SSID, KEY_SLOTS};

#[derive(Debug)]
pub struct SSHConfig {
    pub hostkey: SignKey,

    pub password_authentication: bool,
    pub admin_pw: Option<PwHash>,
    pub admin_keys: [Option<Ed25519PubKey>; KEY_SLOTS],

    /// WiFi
    pub wifi_ssid: String<32>,
    pub wifi_pw: Option<String<63>>,

    /// Networking
    /// TODO: Populate this field from esp's hardware info or just refer it from HAL?
    /// Only intended purpose I see for keeping it here is for spoofing?
    pub mac: [u8; 6],
    /// `None` for DHCP
    pub ip4_static: Option<StaticConfigV4>,

    /// UART
    pub uart_pins: SerdePinConfig,
}

#[derive(Debug, Clone)]
pub struct SerdePinConfig {
    pub tx: u8,
    pub rx: u8,
    pub rts: Option<u8>,
    pub cts: Option<u8>,
}

impl Default for SerdePinConfig {
    fn default() -> Self {
        Self { 
            tx: 10,
            rx: 11,
            // tx: env!("SSH_STAMP_TX_PIN").parse().unwrap(),
            // rx: env!("SSH_STAMP_RX_PIN").parse().unwrap(),
            rts: option_env!("SSH_STAMP_RTS").map(|s| s.parse().unwrap()),
            cts: option_env!("SSH_STAMP_CTS").map(|s| s.parse().unwrap()),
        }
    }
}

pub struct GPIOConfig {
    pub gpio10: Option<AnyPin<'static>>,
    pub gpio11: Option<AnyPin<'static>>,
}

pub struct PinChannel {
    pub config: SerdePinConfig,
    pub gpios: GPIOConfig,
    pub tx: Channel::<CriticalSectionRawMutex, (), 1>,
    pub rx: Channel::<CriticalSectionRawMutex, (), 1>,
    // TODO: cts/rts pins
}

impl PinChannel {
    pub fn new(config: SerdePinConfig, gpios: GPIOConfig) -> Self {
        Self {
            config,
            gpios,
            tx: Channel::<CriticalSectionRawMutex, (), 1>::new(),
            rx: Channel::<CriticalSectionRawMutex, (), 1>::new(),
        }
    }

    pub async fn recv_tx(&mut self) -> errors::Result<AnyPin<'static>> {
        // tx needs to lock here.
        self.tx.receive().await;

        Ok(match self.config.tx {
            10 => self.gpios.gpio10.take().ok_or_else(|| errors::Error::InvalidPin)?,
            11 => self.gpios.gpio11.take().ok_or_else(|| errors::Error::InvalidPin)?,
            _ => return Err(errors::Error::InvalidPin)
        })
    }

    pub async fn send_tx(&mut self, pin: AnyPin<'static>) -> errors::Result<()> {
        match self.config.tx {
            10 => self.gpios.gpio10 = Some(pin),
            11 => self.gpios.gpio11 = Some(pin),
            _ => return Err(errors::Error::InvalidPin)
        };

        // tx lock needs to be released. 
        self.tx.send(()).await;
        Ok(())
    }

    pub async fn recv_rx(&mut self) -> errors::Result<AnyPin<'static>> {
        // rx needs to lock here.
        self.rx.receive().await;

        Ok(match self.config.rx {
            10 => self.gpios.gpio10.take().ok_or_else(|| errors::Error::InvalidPin)?,
            11 => self.gpios.gpio11.take().ok_or_else(|| errors::Error::InvalidPin)?,
            _ => return Err(errors::Error::InvalidPin)
        })
    }

    pub async fn send_rx(&mut self, pin: AnyPin<'static>) -> errors::Result<()> {
        match self.config.rx {
            10 => self.gpios.gpio10 = Some(pin),
            11 => self.gpios.gpio11 = Some(pin),
            _ => return Err(errors::Error::InvalidPin)
        };

        // rx lock needs to be released. 
        self.rx.send(()).await;
        Ok(())
    }
}


// TODO: Yikes, this struct and resolve_pin() need to be re-thought for the different ICs and dev boards?.. implementing a suitable
// validation function for them and potentially writing a macro that adapts to each PAC (not all ICs have the same number
// of pins).
pub struct PinConfig {
    pub tx: AnyPin<'static>,
    pub rx: AnyPin<'static>,
    // pub gpio0: Option<peripherals::GPIO0<'a>>,
    // pub gpio1: Option<peripherals::GPIO1<'a>>,
    // pub gpio2: Option<peripherals::GPIO2<'a>>,
    // pub gpio3: Option<peripherals::GPIO3<'a>>,
    // pub gpio4: Option<peripherals::GPIO4<'a>>,
    // pub gpio5: Option<peripherals::GPIO5<'a>>,
    // pub gpio6: Option<peripherals::GPIO6<'a>>,
    // pub gpio7: Option<peripherals::GPIO7<'a>>,
    // pub gpio8: Option<peripherals::GPIO8<'a>>,
    // pub gpio9: Option<peripherals::GPIO9<'a>>,
    // pub gpio10: Option<peripherals::GPIO10<'a>>,
    // pub gpio11: Option<peripherals::GPIO11<'a>>,
    // pub gpio12: Option<peripherals::GPIO12<'a>>,
    // pub gpio13: Option<peripherals::GPIO13<'a>>,
    // pub gpio14: Option<peripherals::GPIO14<'a>>,
    // pub gpio15: Option<peripherals::GPIO15<'a>>,
    // pub gpio16: Option<peripherals::GPIO16<'a>>,
    // pub gpio17: Option<peripherals::GPIO17<'a>>,
    // pub gpio18: Option<peripherals::GPIO18<'a>>,
    // pub gpio19: Option<peripherals::GPIO19<'a>>,
    // pub gpio20: Option<peripherals::GPIO20<'a>>,
    // pub gpio21: Option<peripherals::GPIO21<'a>>,
    // pub gpio22: Option<peripherals::GPIO22<'a>>,
    // pub gpio23: Option<peripherals::GPIO23<'a>>,
    // pub gpio24: Option<peripherals::GPIO24<'a>>,
    // pub gpio25: Option<peripherals::GPIO25<'a>>,
    // pub gpio26: Option<peripherals::GPIO26<'a>>,
    // pub gpio27: Option<peripherals::GPIO27<'a>>,
    // pub gpio28: Option<peripherals::GPIO28<'a>>,
    // pub gpio29: Option<peripherals::GPIO29<'a>>,
    // pub gpio30: Option<peripherals::GPIO30<'a>>,
}

pub struct PinConfigAlt {
    // pub gpio0: Option<AnyPin<'a>>,
    // pub gpio1: Option<AnyPin<'a>>,
    pub peripherals: peripherals::Peripherals,
}

impl PinConfigAlt {
    pub fn new(peripherals: peripherals::Peripherals) -> Self {
        Self {
            // gpio0: Some(peripherals.GPIO0.into()),
            // gpio1: Some(peripherals.GPIO1.into()),
                        peripherals,
        }
    }

    pub fn take_pin<'a>(&'a mut self, pin: u8) -> AnyPin<'a> {
        match pin {
            0 => self.peripherals.GPIO0.reborrow().into(),
            1 => self.peripherals.GPIO1.reborrow().into(),
            _ => panic!(),
        }
    }

    // pub fn give_pin(&'a mut self, peripherals: &'a mut peripherals::Peripherals, pin: u8) {
    //     match pin {
    //         0 => self.gpio0 = Some(peripherals.GPIO0.reborrow().into()),
    //         1 => self.gpio1 = Some(peripherals.GPIO1.reborrow().into()),
    //         _ => panic!(),
    //     }
    // }
}

impl PinConfig {
    pub fn new(mut gpio_config: GPIOConfig, config_inner: SerdePinConfig) -> errors::Result<Self> {
        if config_inner.rx == config_inner.tx {
            return Err(errors::Error::InvalidPin);
        }
        
        // SAFETY: Safe because moved in peripherals.
        Ok(Self {
            rx: match config_inner.rx {
                10 =>  gpio_config.gpio10.take().unwrap().into(),
                11 =>  gpio_config.gpio11.take().unwrap().into(),
                _ => return Err(errors::Error::InvalidPin),
            },
            tx: match config_inner.tx {
                10 => gpio_config.gpio10.take().unwrap().into(),
                11 => gpio_config.gpio11.take().unwrap().into(),
                _ => return Err(errors::Error::InvalidPin),
            }
        })
    }

    // pub fn tx(&mut self) -> errors::Result<SunsetMutex<AnyPin<'_>>> {
    //     Ok(SunsetMutex::new(self.take_pin(self.pin_config_inner.tx)?))
    // }
    
    // pub fn rx(&mut self) -> errors::Result<SunsetMutex<AnyPin<'_>>> {
    //     Ok(SunsetMutex::new(self.take_pin(self.pin_config_inner.rx)?))
    // }

    // pub fn rts(&mut self) -> errors::Result<Option<SunsetMutex<AnyPin<'_>>>> {
    //     self.pin_config_inner.rts.map(|rts| Ok(SunsetMutex::new(self.take_pin(rts)?))).transpose()
    // }

    // pub fn cts(&mut self) -> errors::Result<Option<SunsetMutex<AnyPin<'_>>>> {
    //     self.pin_config_inner.cts.map(|cts| Ok(SunsetMutex::new(self.take_pin(cts)?))).transpose()
    // }

    // Resolves a u8 pin number into an AnyPin GPIO type.
    // Returns None if the pin number is invalid or unsupported.
    // pub fn give_pin(&mut self, pin_num: u8, peripherals: &'a mut Peripherals) -> errors::Result<()> {
    //     match pin_num {
    //         0 => self.gpio0 = Some(peripherals.GPIO0.reborrow()),
    //         1 => self.gpio1 = Some(peripherals.GPIO1.reborrow()),
    //         2 => self.gpio2 = Some(peripherals.GPIO2.reborrow()),
    //         3 => self.gpio3 = Some(peripherals.GPIO3.reborrow()),
    //         4 => self.gpio4 = Some(peripherals.GPIO4.reborrow()),
    //         5 => self.gpio5 = Some(peripherals.GPIO5.reborrow()),
    //         6 => self.gpio6 = Some(peripherals.GPIO6.reborrow()),
    //         7 => self.gpio7 = Some(peripherals.GPIO7.reborrow()),
    //         8 => self.gpio8 = Some(peripherals.GPIO8.reborrow()),
    //         9 => self.gpio9 = Some(peripherals.GPIO9.reborrow()),
    //         10 => self.gpio10 = Some(peripherals.GPIO10.reborrow()),
    //         11 => self.gpio11 = Some(peripherals.GPIO11.reborrow()),
    //         12 => self.gpio12 = Some(peripherals.GPIO12.reborrow()),
    //         13 => self.gpio13 = Some(peripherals.GPIO13.reborrow()),
    //         14 => self.gpio14 = Some(peripherals.GPIO14.reborrow()),
    //         15 => self.gpio15 = Some(peripherals.GPIO15.reborrow()),
    //         16 => self.gpio16 = Some(peripherals.GPIO16.reborrow()),
    //         17 => self.gpio17 = Some(peripherals.GPIO17.reborrow()),
    //         18 => self.gpio18 = Some(peripherals.GPIO18.reborrow()),
    //         19 => self.gpio19 = Some(peripherals.GPIO19.reborrow()),
    //         20 => self.gpio20 = Some(peripherals.GPIO20.reborrow()),
    //         21 => self.gpio21 = Some(peripherals.GPIO21.reborrow()),
    //         22 => self.gpio22 = Some(peripherals.GPIO22.reborrow()),
    //         23 => self.gpio23 = Some(peripherals.GPIO23.reborrow()),
    //         24 => self.gpio24 = Some(peripherals.GPIO24.reborrow()),
    //         25 => self.gpio25 = Some(peripherals.GPIO25.reborrow()),
    //         26 => self.gpio26 = Some(peripherals.GPIO26.reborrow()),
    //         27 => self.gpio27 = Some(peripherals.GPIO27.reborrow()),
    //         28 => self.gpio28 = Some(peripherals.GPIO28.reborrow()),
    //         29 => self.gpio29 = Some(peripherals.GPIO29.reborrow()),
    //         30 => self.gpio30 = Some(peripherals.GPIO30.reborrow()),
    //         _ => return Err(errors::Error::InvalidPin),
    //     }
    //     Ok(())
    // }

    // generate_gpio_functions!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30);

    /// Resolves a u8 pin number into an AnyPin GPIO type.
    /// Returns None if the pin number is invalid or unsupported.
    pub fn initialize_pin(peripherals: peripherals::Peripherals, pin_number: u8) -> errors::Result<AnyPin<'static>> {
        match pin_number {
            0 => Ok(peripherals.GPIO0.into()),

            _ => Err(errors::Error::InvalidPin),
        }
    }

    // /// Resolves a u8 pin number into an AnyPin GPIO type.
    // /// Returns None if the pin number is invalid or unsupported.
    // pub fn take_pin(&mut self, pin_num: u8) -> errors::Result<AnyPin<'a>> {
    //     match pin_num {
    //         0 => self.gpio0.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         1 => self.gpio1.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         2 => self.gpio2.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         3 => self.gpio3.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         4 => self.gpio4.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         5 => self.gpio5.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         6 => self.gpio6.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         7 => self.gpio7.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         8 => self.gpio8.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         9 => self.gpio9.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         10 => self.gpio10.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         11 => self.gpio11.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         12 => self.gpio12.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         13 => self.gpio13.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         14 => self.gpio14.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         15 => self.gpio15.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         16 => self.gpio16.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         17 => self.gpio17.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         18 => self.gpio18.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         19 => self.gpio19.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         20 => self.gpio20.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         21 => self.gpio21.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         22 => self.gpio22.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         23 => self.gpio23.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         24 => self.gpio24.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         25 => self.gpio25.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         26 => self.gpio26.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         27 => self.gpio27.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         28 => self.gpio28.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         29 => self.gpio29.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         30 => self.gpio30.take().map(AnyPin::from).ok_or(errors::Error::InvalidPin),
    //         _ => Err(errors::Error::InvalidPin),
    //     }
    // }
}

// // TODO: Revisit this and compare them with esp-hal examples, see what they use for their HIL nowadays.
// impl Default for PinConfig {
//     fn default() -> Self {
//         let rx = SunsetMutex::new(PinConfig::resolve_pin(10).expect("Invalid RX pin"));
//         let tx = SunsetMutex::new(PinConfig::resolve_pin(11).expect("Invalid TX pin"));
//         PinConfig {
//             rx,
//             tx,
//             rts: None,
//             cts: None,
//         }
//     }
// }

impl SSHConfig {
    /// Bump this when the format changes
    pub const CURRENT_VERSION: u8 = 6;

    /// Creates a new config with default parameters.
    ///
    /// Will only fail on RNG failure.
    pub fn new() -> Result<Self> {
        let hostkey = SignKey::generate(KeyType::Ed25519, None)?;
        let wifi_ssid: String<32> =
            option_env!("WIFI_SSID").unwrap_or(DEFAULT_SSID).try_into().trap()?;
        let wifi_pw: Option<String<63>> =
            option_env!("WIFI_PW").map(|s| s.try_into()).transpose().trap()?;
        let mac = random_mac()?;

        let uart_pins = SerdePinConfig::default();

        Ok(SSHConfig {
            hostkey,
            password_authentication: true,
            admin_pw: None,
            admin_keys: Default::default(),
            wifi_ssid,
            wifi_pw,
            mac,
            ip4_static: None,
            uart_pins,
        })
    }

    pub fn set_admin_pw(&mut self, pw: Option<&str>) -> Result<()> {
        self.admin_pw = pw.map(|p| PwHash::new(p)).transpose()?;
        Ok(())
    }

    pub fn check_admin_pw(&mut self, pw: &str) -> bool {
        if let Some(ref p) = self.admin_pw {
            p.check(pw)
        } else {
            false
        }
    }
}

fn random_mac() -> Result<[u8; 6]> {
    let mut mac = [0u8; 6];
    sunset::random::fill_random(&mut mac)?;
    // unicast, locally administered
    mac[0] = (mac[0] & 0xfc) | 0x02;
    Ok(mac)
}

// a private encoding specific to demo config, not SSH defined.
fn enc_signkey(k: &SignKey, s: &mut dyn SSHSink) -> WireResult<()> {
    // need to add a variant field if we support more key types.
    match k {
        SignKey::Ed25519(k) => k.to_bytes().enc(s),
        _ => Err(WireError::UnknownVariant),
    }
}

fn dec_signkey<'de, S>(s: &mut S) -> WireResult<SignKey>
where
    S: SSHSource<'de>,
{
    let k: ed25519_dalek::SecretKey = SSHDecode::dec(s)?;
    let k = ed25519_dalek::SigningKey::from_bytes(&k);
    Ok(SignKey::Ed25519(k))
}

// encode Option<T> as a bool then maybe a value
fn enc_option<T: SSHEncode>(v: &Option<T>, s: &mut dyn SSHSink) -> WireResult<()> {
    v.is_some().enc(s)?;
    v.enc(s)
}

fn dec_option<'de, S, T: SSHDecode<'de>>(s: &mut S) -> WireResult<Option<T>>
where
    S: SSHSource<'de>,
{
    bool::dec(s)?.then(|| SSHDecode::dec(s)).transpose()
}

fn enc_ip4config(v: &Option<StaticConfigV4>, s: &mut dyn SSHSink) -> WireResult<()> {
    v.is_some().enc(s)?;
    if let Some(v) = v {
        v.address.address().to_bits().enc(s)?;
        v.address.prefix_len().enc(s)?;
        // to u32
        let gw = v.gateway.map(|a| a.to_bits());
        enc_option(&gw, s)?;
    }
    Ok(())
}

fn dec_ip4config<'de, S>(s: &mut S) -> WireResult<Option<StaticConfigV4>>
where
    S: SSHSource<'de>,
{
    let opt = bool::dec(s)?;
    opt.then(|| {
        let ad: u32 = SSHDecode::dec(s)?;
        let ad = Ipv4Addr::from_bits(ad);
        let prefix = SSHDecode::dec(s)?;
        if prefix > 32 {
            // emabassy panics, so test it here
            return Err(WireError::PacketWrong);
        }
        let gw: Option<u32> = dec_option(s)?;
        let gateway = gw.map(|gw| Ipv4Addr::from_bits(gw));
        Ok(StaticConfigV4 {
            address: Ipv4Cidr::new(ad, prefix),
            gateway,
            dns_servers: Vec::new(),
        })
    })
    .transpose()
}

fn dec_uart_pins<'de, S>(s: &mut S) -> WireResult<SerdePinConfig>
where
    S: SSHSource<'de>,
{
    let tx = u8::dec(s)?;
    let rx = u8::dec(s)?;
    let rts = dec_option(s)?;
    let cts = dec_option(s)?;
    Ok(SerdePinConfig { tx, rx, rts, cts })
}

impl SSHEncode for SSHConfig {
    fn enc(&self, s: &mut dyn SSHSink) -> WireResult<()> {
        println!("enc si");
        enc_signkey(&self.hostkey, s)?;
        enc_option(&self.admin_pw, s)?;

        for k in self.admin_keys.iter() {
            enc_option(k, s)?;
        }

        self.wifi_ssid.as_str().enc(s)?;
        enc_option(&self.wifi_pw, s)?;

        self.mac.enc(s)?;

        enc_ip4config(&self.ip4_static, s)?;

        Ok(())
    }
}

impl<'de> SSHDecode<'de> for SSHConfig {
    fn dec<S>(s: &mut S) -> WireResult<Self>
    where
        S: SSHSource<'de>,
    {
        let hostkey = dec_signkey(s)?;

        let admin_pw = dec_option(s)?;

        let mut admin_keys = [None, None, None];
        for k in admin_keys.iter_mut() {
            *k = dec_option(s)?;
        }

        let wifi_ssid = SSHDecode::dec(s)?;
        let wifi_pw = dec_option(s)?;

        let mac = SSHDecode::dec(s)?;

        let ip4_static = dec_ip4config(s)?;

        // Decode password_authentication (missing in original code)
        let password_authentication = SSHDecode::dec(s)?;

        let uart_pins = dec_uart_pins(s)?;

        Ok(Self {
            hostkey,
            password_authentication,
            admin_pw,
            admin_keys,
            wifi_ssid,
            wifi_pw,
            mac,
            ip4_static,
            uart_pins,
        })
    }
}

/// Stores a bcrypt password hash.
///
/// We use bcrypt because it seems the best password hashing option where
/// memory hardness isn't possible (the rp2040 is smaller than CPU or GPU memory).
///
/// The cost is currently set to 6, taking ~500ms on a 125mhz rp2040.
/// Time converges to roughly 8.6ms * 2**cost
///
/// Passwords are pre-hashed to avoid bcrypt's 72 byte limit.
/// rust-bcrypt allows nulls in passwords.
/// We use an hmac rather than plain hash to avoid password shucking
/// (an attacker bcrypts known hashes from some other breach, then
/// brute forces the weaker hash for any that match).
//#[derive(Clone, SSHEncode, SSHDecode, PartialEq)]
#[derive(Clone, PartialEq)]
pub struct PwHash {
    salt: [u8; 16],
    hash: [u8; 24],
    cost: u8,
}

impl PwHash {
    const COST: u8 = 6;
    /// `pw` must not be empty.
    pub fn new(pw: &str) -> Result<Self> {
        if pw.is_empty() {
            return sunset::error::BadUsage.fail();
        }

        let mut salt = [0u8; 16];
        sunset::random::fill_random(&mut salt)?;
        let prehash = Self::prehash(pw, &salt);
        let cost = Self::COST;
        let hash = bcrypt::bcrypt(cost as u32, salt, &prehash);
        Ok(Self { salt, hash, cost })
    }

    pub fn check(&self, pw: &str) -> bool {
        if pw.is_empty() {
            return false;
        }
        let prehash = Self::prehash(pw, &self.salt);
        let check_hash = bcrypt::bcrypt(self.cost as u32, self.salt.clone(), &prehash);
        check_hash.ct_eq(&self.hash).into()
    }

    fn prehash(pw: &str, salt: &[u8]) -> [u8; 32] {
        // OK unwrap: can't fail, accepts any length
        // TODO: Generalise, not only Espressif esp_hal
        let mut prehash = Hmac::<Sha256>::new_from_slice(&salt).unwrap();
        prehash.update(pw.as_bytes());
        prehash.finalize().into_bytes().into()
    }
}

impl core::fmt::Debug for PwHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PwHash").finish_non_exhaustive()
    }
}

impl SSHEncode for PwHash {
    fn enc(&self, s: &mut dyn SSHSink) -> WireResult<()> {
        self.salt.enc(s)?;
        self.hash.enc(s)?;
        self.cost.enc(s)
    }
}

impl<'de> SSHDecode<'de> for PwHash {
    fn dec<S>(s: &mut S) -> WireResult<Self>
    where
        S: SSHSource<'de>,
    {
        let salt = <[u8; 16]>::dec(s)?;
        let hash = <[u8; 24]>::dec(s)?;
        let cost = u8::dec(s)?;
        Ok(PwHash { salt, hash, cost })
    }
}