use core::net::Ipv4Addr;
use embassy_net::{Ipv4Cidr, StaticConfigV4};
use embedded_storage::nor_flash::ReadNorFlash;
use esp_storage::FlashStorage;
use heapless::{String, Vec};

use esp_println::println;

use bcrypt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use sunset::error::TrapBug;
use sunset::{Error, KeyType, Result};
use sunset::{
    packets::Ed25519PubKey,
    sshwire::{SSHDecode, SSHEncode, SSHSink, SSHSource, WireError, WireResult},
    SignKey,
};

use crate::settings::{KEY_SLOTS, SSH_SERVER_ID};
use crate::storage::{CONFIG_AREA_SIZE, CONFIG_OFFSET};

#[derive(Debug, Clone, PartialEq)]
pub struct SSHConfig {
    pub hostkey: SignKey,

    pub password_authentication: bool,
    pub admin_pw: Option<PwHash>,
    pub admin_keys: [Option<Ed25519PubKey>; KEY_SLOTS],

    /// WiFi SSID
    pub wifi_ssid: String<32>,
    /// WPA2 passphrase. None is Open network.
    pub wifi_pw: Option<String<63>>,

    /// TODO: Populate this field from esp's hardware info or just refer it from HAL?
    /// Only intended purpose I see for keeping it here is for spoofing?
    pub mac: [u8; 6],

    /// `None` for DHCP
    pub ip4_static: Option<StaticConfigV4>,
}

impl SSHConfig {
    /// Bump this when the format changes
    /// TODO: Revise this, unclear if this is needed with sequential_storage internal map() representation
    pub const CURRENT_VERSION: u8 = 6;

    /// Creates a new config with default parameters.
    ///
    /// Will only fail on RNG failure.
    pub fn new() -> Result<Self, Error> {
        let hostkey = SignKey::generate(KeyType::Ed25519, None)?;
        let wifi_ssid: String<32> =
            option_env!("WIFI_SSID").unwrap_or(SSH_SERVER_ID).try_into().trap()?;
        let wifi_pw: Option<String<63>> =
            option_env!("WIFI_PW").map(|s| s.try_into()).transpose().trap()?;
        let mac = random_mac()?;

        Ok(SSHConfig {
            hostkey,
            password_authentication: true,
            admin_pw: None,
            admin_keys: Default::default(),
            wifi_ssid,
            wifi_pw,
            mac,
            ip4_static: None,
        })
    }

    pub fn set_admin_pw(&mut self, pw: Option<&str>) -> Result<Self, Error> {
        self.admin_pw = pw.map(|p| PwHash::new(p)).transpose()?;
        Ok(self.clone())
    }

    pub fn check_admin_pw(&mut self, pw: &str) -> Result<bool, Error> {
        if let Some(ref p) = self.admin_pw {
            Ok(p.check(pw))
        } else {
            Ok(false)
        }
    }

    /// Loads (deserialises) SSHConfig from FlashStorage
    pub fn load(flash: &FlashStorage) -> Result<Self, Error> {
        let mut config = [0u8; CONFIG_AREA_SIZE];
        flash.read(CONFIG_OFFSET, &mut config);

        let config = SSHConfig::serialise();

        Ok(config)
    }

    /// Saves (serialises) SSHConfig to FlashStorage
    pub fn save(config: SSHConfig) -> Result<SSHConfig, Error> {
        let mut source = sunset::sshwire::SliceSource::new(config);
        SSHConfig::dec(&mut source).map_err(|_| Error::Format)
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

impl SSHEncode for SSHConfig {
    fn enc(&self, s: &mut dyn SSHSink) -> WireResult<()> {
        println!("enc si");
        enc_signkey(&self.hostkey, s)?;

        // enc_option(&self.console_pw, s)?;

        // for k in self.console_keys.iter() {
        //     enc_option(k, s)?;
        // }

        // self.console_noauth.enc(s)?;

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

        Ok(Self {
            hostkey,
            password_authentication,
            admin_pw,
            admin_keys,
            wifi_ssid,
            wifi_pw,
            mac,
            ip4_static,
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