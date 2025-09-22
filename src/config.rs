use core::net::Ipv4Addr;
use embassy_net::{Ipv4Cidr, StaticConfigV4, StaticConfigV6};
use esp_hal::gpio::AnyPin;
use esp_hal::peripherals;
use heapless::{String, Vec};

use esp_println::dbg;

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
    pub ipv4_static: Option<StaticConfigV4>,
    pub ipv6_static: Option<StaticConfigV6>,
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
            // TODO: This env comes from SSH env events/packets, not from system's std::env / core::env (if any)... so it shouldn't be unsafe()
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
        //self.tx.receive().await;

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
        let res = Ok(match self.config.rx {
            10 => self.gpios.gpio10.take().ok_or_else(|| errors::Error::InvalidPin)?,
            11 => self.gpios.gpio11.take().ok_or_else(|| errors::Error::InvalidPin)?,
            _ => return Err(errors::Error::InvalidPin)
        });
        dbg!("recv_rx: no channel receive");
        // rx needs to lock here.
        // dbg!("recv_rx: before rx.receive.await");
        // self.rx.receive().await;
        // dbg!("recv_rx: after rx.receive.await");

        res
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

    pub async fn with_channel<F>(&mut self, f: F) -> errors::Result<()> 
    where F: for<'a> AsyncFnOnce(AnyPin<'a>, AnyPin<'a>) {
        dbg!("inner: with_channel begin, recv_rx call");
        let mut rx = self.recv_rx().await?;
        dbg!("inner: with_channel recv_tx call");
        let mut tx = self.recv_tx().await?;

        dbg!("inner: with_channel f-reborrow");
        f(rx.reborrow(), tx.reborrow()).await;

        dbg!("inner: with_channel, before send{rx/tx}");
        self.send_rx(rx).await.unwrap();
        self.send_tx(tx).await.unwrap();

        Ok(())
    }
}


// TODO: This struct and resolve_pin() need to be re-thought for the different ICs and dev boards?.. implementing a suitable
// validation function for them and potentially writing a macro that adapts to each PAC (not all ICs have the same number
// of pins).
pub struct PinConfig {
    pub tx: AnyPin<'static>,
    pub rx: AnyPin<'static>,
}

pub struct PinConfigAlt {
    pub peripherals: peripherals::Peripherals,
}

impl PinConfigAlt {
    pub fn new(peripherals: peripherals::Peripherals) -> Self {
        Self {
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

    /// Resolves a u8 pin number into an AnyPin GPIO type.
    /// Returns None if the pin number is invalid or unsupported.
    pub fn initialize_pin(peripherals: peripherals::Peripherals, pin_number: u8) -> errors::Result<AnyPin<'static>> {
        match pin_number {
            0 => Ok(peripherals.GPIO0.into()),

            _ => Err(errors::Error::InvalidPin),
        }
    }
}

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
            ipv4_static: None,
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

fn enc_ipv4_config(v: &Option<StaticConfigV4>, s: &mut dyn SSHSink) -> WireResult<()> {
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

fn dec_ipv4_config<'de, S>(s: &mut S) -> WireResult<Option<StaticConfigV4>>
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
        enc_signkey(&self.hostkey, s)?;
        enc_option(&self.admin_pw, s)?;

        for k in self.admin_keys.iter() {
            enc_option(k, s)?;
        }

        self.wifi_ssid.as_str().enc(s)?;
        enc_option(&self.wifi_pw, s)?;

        self.mac.enc(s)?;

        enc_ipv4_config(&self.ipv4_static, s)?;

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

        let ipv4_static = dec_ipv4_config(s)?;
        let ipv6_static = None; // TODO: Decode ipv6_config

        // Decode password_authentication
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
            ipv4_static,
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