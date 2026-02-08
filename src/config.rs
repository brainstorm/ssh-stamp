use core::net::Ipv4Addr;
#[cfg(feature = "ipv6")]
use core::net::Ipv6Addr;
use embassy_net::{Ipv4Cidr, StaticConfigV4};
#[cfg(feature = "ipv6")]
use embassy_net::{Ipv6Cidr, StaticConfigV6};
use heapless::{String};

use esp_println::dbg;

use bcrypt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use sunset::packets::Ed25519PubKey;
use sunset::{sshwire, KeyType, Result};
use sunset::{
    sshwire::{SSHDecode, SSHEncode, SSHSink, SSHSource, WireError, WireResult},
    SignKey,
};

use crate::settings::{DEFAULT_SSID, DEFAULT_UART_RX_PIN, DEFAULT_UART_TX_PIN, KEY_SLOTS};

#[derive(Debug, PartialEq)]
pub struct SSHStampConfig {
    //pub first_boot: bool,
    pub hostkey: SignKey,

    /// Authentication
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
    pub ipv4_static: Option<StaticConfigV4>,
    #[cfg(feature = "ipv6")]
    pub ipv6_static: Option<StaticConfigV6>,
    /// UART
    pub uart_pins: UartPins,
}

#[derive(Debug, PartialEq)]
pub struct UartPins {
    rx: u8,
    tx: u8,
}

impl Default for UartPins {
    fn default() -> Self {
        // sensible defaults for UART pins; adjust if your board uses different pins
        UartPins { rx: DEFAULT_UART_RX_PIN, tx: DEFAULT_UART_TX_PIN }
    }
}

impl SSHStampConfig {
    /// Bump this when the format changes
    pub const CURRENT_VERSION: u8 = 6;

    /// Creates a new config with default parameters.
    ///
    /// Will only fail on RNG failure.
    pub fn new() -> Result<Self> {
        let hostkey = SignKey::generate(KeyType::Ed25519, None)?;

        // TODO: Those env events come from system's std::env / core::env (if any)... so it shouldn't be unsafe()
        let wifi_ssid = Self::default_ssid();
        let mac = random_mac()?;
        let wifi_pw = None;

        let uart_pins = UartPins::default();

        Ok(SSHStampConfig {
            hostkey,
            password_authentication: true,
            admin_pw: None,
            admin_keys: Default::default(),
            wifi_ssid,
            wifi_pw,
            mac,
            ipv4_static: None,
            #[cfg(feature = "ipv6")]
            ipv6_static: None,
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

    pub(crate) fn default_ssid() -> String<32> {
        let mut s = String::<32>::new();
        s.push_str(DEFAULT_SSID).unwrap();
        s
    }

    // pub fn config_change(&mut self, conf: SSHConfig) -> Result<()> {
    //      ServEvent::ConfigChange();
    // }
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
pub(crate) fn enc_option<T: SSHEncode>(v: &Option<T>, s: &mut dyn SSHSink) -> WireResult<()> {
    v.is_some().enc(s)?;
    v.enc(s)
}

pub(crate) fn dec_option<'de, S, T: SSHDecode<'de>>(s: &mut S) -> WireResult<Option<T>>
where
    S: SSHSource<'de>,
{
    bool::dec(s)?.then(|| SSHDecode::dec(s)).transpose()
}

// encode Option<heapless::String<N>> as a bool then the &str contents (heapless::String doesn't implement SSHEncode)
pub(crate) fn enc_option_str<const N: usize>(v: &Option<String<N>>, s: &mut dyn SSHSink) -> WireResult<()> {
    v.is_some().enc(s)?;
    if let Some(ref st) = v {
        st.as_str().enc(s)?;
    }
    Ok(())
}

fn enc_ipv4_config(v: &Option<StaticConfigV4>, s: &mut dyn SSHSink) -> WireResult<()> {
    v.is_some().enc(s)?;
    if let Some(v) = v {
        v.address.address().to_bits().enc(s)?;
        dbg!("enc_ipv4_config: prefix", &v.address.prefix_len());
        v.address.prefix_len().enc(s)?;
        // to u32
        let gw = v.gateway.map(|a| a.to_bits());
        enc_option(&gw, s)?;
    }
    Ok(())
}

#[cfg(feature = "ipv6")]
fn enc_ipv6_config(v: &Option<StaticConfigV6>, s: &mut dyn SSHSink) -> WireResult<()> {
    v.is_some().enc(s)?;
    if let Some(v) = v {
        v.address.address().to_bits().enc(s)?;
        v.address.prefix_len().enc(s)?;
        let gw = v.gateway.map(|a| a.to_bits());
        enc_option(&gw, s)?;
    }
    Ok(())
}

fn dec_ipv4_config<'de, S>(s: &mut S) -> WireResult<Option<StaticConfigV4>>
where
    S: SSHSource<'de>,
{
    let opt = bool::dec(s)?;
    opt.then(|| {
        let ad: u32 = SSHDecode::dec(s)?;
        let ad = Ipv4Addr::from_bits(ad);
        let prefix: u8 = SSHDecode::dec(s)?;
        if prefix > 32 {
            // embassy panics, so test it here
            return Err(WireError::PacketWrong);
        }
        let gw: Option<u32> = dec_option(s)?;
        let gateway = gw.map(|gw| Ipv4Addr::from_bits(gw));
        Ok(StaticConfigV4 {
            address: Ipv4Cidr::new(ad, prefix),
            gateway,
            dns_servers: Default::default(),
        })
    })
    .transpose()
}

#[cfg(feature = "ipv6")]
fn dec_ipv6_config<'de, S>(s: &mut S) -> WireResult<Option<StaticConfigV6>>
where
    S: SSHSource<'de>,
{
    let opt = bool::dec(s)?;
    opt.then(|| {
        let ad: u128 = SSHDecode::dec(s)?;
        let ad = Ipv6Addr::from_bits(ad);
        let prefix = SSHDecode::dec(s)?;
        if prefix > 32 {
            // embassy panics, so test it here
            return Err(WireError::PacketWrong);
        }
        let gw: Option<u128> = dec_option(s)?;
        let gateway = gw.map(|gw| Ipv6Addr::from_bits(gw));
        Ok(StaticConfigV6 {
            address: Ipv6Cidr::new(ad, prefix),
            gateway,
            dns_servers: Vec::new(),
        })
    })
    .transpose()
}

impl SSHEncode for SSHStampConfig {
    fn enc(&self, s: &mut dyn SSHSink) -> WireResult<()> {
        enc_signkey(&self.hostkey, s)?;

        // Authentication
        self.password_authentication.enc(s)?;
        enc_option(&self.admin_pw, s)?;

        for k in self.admin_keys.iter() {
            enc_option(k, s)?;
        }

        self.wifi_ssid.as_str().enc(s)?;
        enc_option_str::<63>(&self.wifi_pw, s)?;
        self.mac.enc(s)?;

        enc_ipv4_config(&self.ipv4_static, s)?;
        #[cfg(feature = "ipv6")]
        enc_ipv6_config(&self.ipv6_static, s)?;

        // Encode UartPins
        self.uart_pins.rx.enc(s)?;
        self.uart_pins.tx.enc(s)?;

        Ok(())
    }
}

impl<'de> SSHDecode<'de> for SSHStampConfig {
    fn dec<S>(s: &mut S) -> WireResult<Self>
    where
        S: SSHSource<'de>,
    {
        let hostkey = dec_signkey(s)?;

        // Authentication
        let password_authentication = SSHDecode::dec(s)?;
        let admin_pw = dec_option(s)?;
        let mut admin_keys = [None; KEY_SLOTS];
        for k in admin_keys.iter_mut() {
            *k = dec_option(s)?;
        }

        let wifi_ssid = SSHDecode::dec(s)?;
        let wifi_pw = dec_option(s)?;
        
        let mac = SSHDecode::dec(s)?;

        let ipv4_static = dec_ipv4_config(s)?;
        #[cfg(feature = "ipv6")]
        let ipv6_static = dec_ipv6_config(s)?;

        // Not supported by sshwire-derive nor virtue (no Option<u8> support)
        // let uart_pins = SSHDecode::dec(s)?;
        let rx: u8 = SSHDecode::dec(s)?;
        let tx: u8 = SSHDecode::dec(s)?;
        let uart_pins = UartPins { rx, tx };

        Ok(Self {
            hostkey,
            password_authentication,
            admin_pw,
            admin_keys,
            wifi_ssid,
            wifi_pw,
            mac,
            ipv4_static,
            #[cfg(feature = "ipv6")]
            ipv6_static,
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

pub fn roundtrip_config() {
    // default config
    let c1 = SSHStampConfig::new().unwrap();
    let mut buf = [0u8; 1000];
    let l = sshwire::write_ssh(&mut buf, &c1).unwrap();
    let v = &buf[..l];
    let c2: SSHStampConfig = sshwire::read_ssh(v, None).unwrap();
    assert_eq!(c1, c2);
}
