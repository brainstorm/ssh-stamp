use core::net::Ipv4Addr;
use embassy_net::{Ipv4Cidr, StaticConfigV4};
use esp_hal::gpio::AnyPin;
use esp_hal::peripherals::Peripherals;
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
use sunset_async::SunsetMutex;

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

pub struct PinConfig<'a> {
    pub pin_config_inner: SerdePinConfig,
    pub peripherals: &'a mut Peripherals,
}

impl<'a> PinConfig<'a> {
    pub fn new(peripherals: &'a mut Peripherals, pin_config_inner: SerdePinConfig) -> Self {
        Self {
            pin_config_inner,
            peripherals
        }
    }

    pub fn tx(&mut self) -> errors::Result<SunsetMutex<AnyPin<'_>>> {
        Ok(SunsetMutex::new(Self::resolve_pin(self.pin_config_inner.tx, self.peripherals)?))
    }
    
    pub fn rx(&mut self) -> errors::Result<SunsetMutex<AnyPin<'_>>> {
        Ok(SunsetMutex::new(Self::resolve_pin(self.pin_config_inner.rx, self.peripherals)?))
    }

    pub fn rts(&mut self) -> errors::Result<Option<SunsetMutex<AnyPin<'_>>>> {
        self.pin_config_inner.rts.map(|rts| Ok(SunsetMutex::new(Self::resolve_pin(rts, self.peripherals)?))).transpose()
    }

    pub fn cts(&mut self) -> errors::Result<Option<SunsetMutex<AnyPin<'_>>>> {
        self.pin_config_inner.cts.map(|cts| Ok(SunsetMutex::new(Self::resolve_pin(cts, self.peripherals)?))).transpose()
    }

    /// Resolves a u8 pin number into an AnyPin GPIO type.
    /// Returns None if the pin number is invalid or unsupported.
    pub fn resolve_pin(pin_num: u8, peripherals: &mut Peripherals) -> errors::Result<AnyPin<'_>> {
        match pin_num {
            0 => Ok(peripherals.GPIO0.reborrow().into()),
            1 => Ok(peripherals.GPIO1.reborrow().into()),
            2 => Ok(peripherals.GPIO2.reborrow().into()),
            3 => Ok(peripherals.GPIO3.reborrow().into()),
            4 => Ok(peripherals.GPIO4.reborrow().into()),
            5 => Ok(peripherals.GPIO5.reborrow().into()),
            6 => Ok(peripherals.GPIO6.reborrow().into()),
            7 => Ok(peripherals.GPIO7.reborrow().into()),
            8 => Ok(peripherals.GPIO8.reborrow().into()),
            9 => Ok(peripherals.GPIO9.reborrow().into()),
            10 => Ok(peripherals.GPIO10.reborrow().into()),
            11 => Ok(peripherals.GPIO11.reborrow().into()),
            12 => Ok(peripherals.GPIO12.reborrow().into()),
            13 => Ok(peripherals.GPIO13.reborrow().into()),
            14 => Ok(peripherals.GPIO14.reborrow().into()),
            15 => Ok(peripherals.GPIO15.reborrow().into()),
            16 => Ok(peripherals.GPIO16.reborrow().into()),
            17 => Ok(peripherals.GPIO17.reborrow().into()),
            18 => Ok(peripherals.GPIO18.reborrow().into()),
            19 => Ok(peripherals.GPIO19.reborrow().into()),
            20 => Ok(peripherals.GPIO20.reborrow().into()),
            21 => Ok(peripherals.GPIO21.reborrow().into()),
            22 => Ok(peripherals.GPIO22.reborrow().into()),
            23 => Ok(peripherals.GPIO23.reborrow().into()),
            24 => Ok(peripherals.GPIO24.reborrow().into()),
            25 => Ok(peripherals.GPIO25.reborrow().into()),
            26 => Ok(peripherals.GPIO26.reborrow().into()),
            27 => Ok(peripherals.GPIO27.reborrow().into()),
            28 => Ok(peripherals.GPIO28.reborrow().into()),
            29 => Ok(peripherals.GPIO29.reborrow().into()),
            30 => Ok(peripherals.GPIO30.reborrow().into()),
            _ => Err(errors::Error::InvalidPin),
        }
    }
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