use log::{debug, warn};

use core::net::Ipv4Addr;
#[cfg(feature = "ipv6")]
use core::net::Ipv6Addr;
use core::str::FromStr;
use embassy_net::{Ipv4Cidr, StaticConfigV4};
#[cfg(feature = "ipv6")]
use embassy_net::{Ipv6Cidr, StaticConfigV6};
use heapless::String;

use sunset::packets::Ed25519PubKey;
use sunset::{KeyType, Result};
use sunset::{
    SignKey,
    sshwire::{SSHDecode, SSHEncode, SSHSink, SSHSource, WireError, WireResult},
};

use crate::errors::Error;
use crate::settings::{
    DEFAULT_SSID, DEFAULT_UART_RX_PIN, DEFAULT_UART_TX_PIN, KEY_SLOTS, WIFI_PASSWORD_CHARS,
};

#[derive(Debug, PartialEq)]
pub struct SSHStampConfig {
    pub hostkey: SignKey,

    /// Authentication: only pubkey-based auth supported
    pub pubkeys: [Option<Ed25519PubKey>; KEY_SLOTS],

    /// WiFi
    pub wifi_ssid: String<32>,
    pub wifi_pw: Option<String<63>>, // Max 64 characters including null-terminator?

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
    /// True until a pubkey is provisioned. Further changes require authentication.
    pub first_login: bool,
}

#[derive(Debug, PartialEq)]
pub struct UartPins {
    pub rx: u8,
    pub tx: u8,
}

impl Default for UartPins {
    fn default() -> Self {
        // sensible defaults for UART pins; adjust if your board uses different pins
        UartPins {
            rx: DEFAULT_UART_RX_PIN,
            tx: DEFAULT_UART_TX_PIN,
        }
    }
}

impl SSHStampConfig {
    /// Bump this when the format changes
    pub const CURRENT_VERSION: u8 = 8;

    /// Creates a new config with default parameters.
    ///
    /// Will only fail on RNG failure.
    pub fn new() -> Result<Self> {
        let hostkey = SignKey::generate(KeyType::Ed25519, None)?;

        let wifi_ssid = Self::default_ssid();
        let mac = random_mac()?;
        let wifi_pw = Some(Self::generate_wifi_password()?);

        let uart_pins = UartPins::default();
        debug!(
            "SSH Stamp Config new() - RX Pin: {}  TX Pin: {}",
            uart_pins.rx, uart_pins.tx
        );

        Ok(SSHStampConfig {
            hostkey,
            pubkeys: Default::default(),
            wifi_ssid,
            wifi_pw,
            mac,
            ipv4_static: None,
            #[cfg(feature = "ipv6")]
            ipv6_static: None,
            uart_pins,
            first_login: true,
        })
    }

    fn generate_wifi_password() -> Result<String<63>> {
        let mut rnd = [0u8; 24];
        sunset::random::fill_random(&mut rnd)?;
        let mut pw = String::<63>::new();
        for &byte in rnd.iter() {
            let _ = pw.push(WIFI_PASSWORD_CHARS[(byte as usize) % 62] as char);
        }
        Ok(pw)
    }

    // Password functions removed; pubkey-only auth supported.

    pub(crate) fn default_ssid() -> String<32> {
        let mut s = String::<32>::new();
        s.push_str(DEFAULT_SSID).unwrap();
        s
    }

    pub(crate) fn add_pubkey(&mut self, key_str: &str) -> Result<(), Error> {
        // Accept OpenSSH public key format (e.g. "ssh-ed25519 AAAA...") and
        // validate it is an Ed25519 key. Insert into the first empty slot or
        // overwrite slot 0 if none empty.

        debug!(
            "Checking pubkey string passed through ENV: {}",
            key_str.trim()
        );

        let openssh = ssh_key::PublicKey::from_str(key_str.trim())?;

        debug!("Public key format valid, continuing to parse");

        match openssh.key_data() {
            ssh_key::public::KeyData::Ed25519(k) => {
                let bytes = k.0; // [u8; 32]
                let newk = Ed25519PubKey {
                    key: sunset::sshwire::Blob(bytes),
                };

                debug!("Parsed Ed25519 public key, adding to config");
                for slot in self.pubkeys.iter_mut() {
                    if slot.is_none() {
                        *slot = Some(newk);
                        return Ok(());
                    }
                }

                warn!("Public key slots full, overwriting the first one");
                // SECURITY: Allow this on FirstAuth ON FIRST BOOT ONLY.
                self.pubkeys[0] = Some(newk);
                Ok(())
            }
            _ => Err(Error::BadKey),
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
pub(crate) fn enc_option_str<const N: usize>(
    v: &Option<String<N>>,
    s: &mut dyn SSHSink,
) -> WireResult<()> {
    v.is_some().enc(s)?;
    if let Some(st) = v {
        st.as_str().enc(s)?;
    }
    Ok(())
}

fn enc_ipv4_config(v: &Option<StaticConfigV4>, s: &mut dyn SSHSink) -> WireResult<()> {
    v.is_some().enc(s)?;
    if let Some(v) = v {
        v.address.address().to_bits().enc(s)?;
        debug!("enc_ipv4_config: prefix = {}", &v.address.prefix_len());
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
        let gateway = gw.map(Ipv4Addr::from_bits);
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

        for k in self.pubkeys.iter() {
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

        // Persist first-login marker
        self.first_login.enc(s)?;

        Ok(())
    }
}

impl<'de> SSHDecode<'de> for SSHStampConfig {
    fn dec<S>(s: &mut S) -> WireResult<Self>
    where
        S: SSHSource<'de>,
    {
        let hostkey = dec_signkey(s)?;

        let mut pubkeys = [None; KEY_SLOTS];
        for k in pubkeys.iter_mut() {
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

        let first_login = SSHDecode::dec(s)?;

        Ok(Self {
            hostkey,
            pubkeys,
            wifi_ssid,
            wifi_pw,
            mac,
            ipv4_static,
            #[cfg(feature = "ipv6")]
            ipv6_static,
            uart_pins,
            first_login,
        })
    }
}
