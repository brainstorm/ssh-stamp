// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Configuration types and serialization.
//!
//! [`SSHStampConfig`] holds all persistent device state: host key, public keys,
//! `WiFi` credentials, MAC address, UART pins and line parameters, and the
//! first-login flag. It is serialized to flash via the `sunset` SSH wire format and
//! deserialized on boot by [`store::load_or_create`](crate::store::load_or_create).
//!
//! On first boot, [`SSHStampConfig::new`] generates a random SSID and WPA2
//! PSK (printed to the serial console).

use log::{debug, warn};

use core::net::Ipv4Addr;
#[cfg(feature = "ipv6")]
use core::net::Ipv6Addr;
use core::str::FromStr;
use embassy_net::{Ipv4Cidr, StaticConfigV4};
#[cfg(feature = "ipv6")]
use embassy_net::{Ipv6Cidr, StaticConfigV6};
use heapless::String;
use ssh_key::PublicKey;
use ssh_key::public::KeyData;
use ssh_stamp_hal::UartParams;

use sunset::packets::Ed25519PubKey;
use sunset::{KeyType, Result};
use sunset::{
    SignKey,
    sshwire::{Blob, SSHDecode, SSHEncode, SSHSink, SSHSource, WireError, WireResult},
};

use crate::errors::Error;
use crate::settings::{KEY_SLOTS, WIFI_PASSWORD_CHARS};

#[derive(Debug, PartialEq)]
pub struct SSHStampConfig {
    pub hostkey: SignKey,

    /// Authentication: only pubkey-based auth supported
    pub pubkeys: [Option<Ed25519PubKey>; KEY_SLOTS],

    /// `WiFi`
    /// Access Point Mode
    pub wifi_ap_ssid: String<32>,
    pub wifi_ap_pw: String<63>,
    /// AP band mode (2.4GHz / 5GHz / Auto). Ignored on chips without 5GHz.
    pub wifi_ap_band: u8,
    /// Station Mode
    pub wifi_sta_ssid: String<32>,
    pub wifi_sta_pw: String<63>,
    /// Networking
    /// MAC address. Special values:
    /// - `[0xFF; 6]`: Generate random MAC on each boot
    /// - Otherwise: Use the stored MAC (defaults to hardware eFuse MAC)
    pub mac: [u8; 6],
    /// `None` for DHCP. Persisted, but nothing applies it to the network
    /// stack yet — AP mode is hardwired to [`settings::DEFAULT_IP`] and
    /// station mode to a DHCP lease.
    ///
    /// [`settings::DEFAULT_IP`]: crate::settings::DEFAULT_IP
    pub ipv4_static: Option<StaticConfigV4>,
    /// Persisted, but likewise never applied, and there is no
    /// `SSH_STAMP_IPV6_*` env var to populate it. See `docs/IPV6.md` for what
    /// the `ipv6` feature covers and what finishing it would take.
    #[cfg(feature = "ipv6")]
    pub ipv6_static: Option<StaticConfigV6>,
    /// UART
    pub uart_pins: UartPins,
    /// UART line parameters (baud, data bits, parity, stop bits) for the
    /// serial bridge. Settable via the `SSH_STAMP_UART_*` env vars.
    pub uart_params: UartParams,
    /// True until a pubkey is provisioned. Further changes require authentication.
    pub first_login: bool,
}

/// UART pin assignment.
///
/// UART TX and RX pin numbers are target-specific and must be provided
/// by the port binary (e.g. `ssh-stamp-esp32`). There is no sensible
/// cross-platform default; `UartPins` is constructed explicitly by the
/// binary and passed to [`SSHStampConfig::new`].
#[derive(Debug, PartialEq)]
pub struct UartPins {
    pub rx: u8,
    pub tx: u8,
}

const MAC_RANDOM_SENTINEL: [u8; 6] = [0xFF; 6];

impl SSHStampConfig {
    /// Bump this when the format changes
    pub const CURRENT_VERSION: u8 = 12;

    /// Check if configured for random MAC on each boot
    #[must_use]
    pub fn is_mac_random(&self) -> bool {
        self.mac == MAC_RANDOM_SENTINEL
    }

    /// Get the MAC address to use (resolves random sentinel)
    /// # Errors
    /// Returns an error if the RNG fails
    pub fn resolve_mac(&self) -> Result<[u8; 6]> {
        if self.is_mac_random() {
            random_mac()
        } else {
            Ok(self.mac)
        }
    }

    /// Creates a new config with default parameters.
    ///
    /// `default_mac` is the MAC the platform wants the device to default to
    /// (typically read from hardware OTP/eFuse). Stored as-is in the config;
    /// may be overwritten later via the `SSH_STAMP_WIFI_MAC_*` env vars.
    ///
    /// `uart_pins` is the TX/RX pin assignment, which is target-specific and
    /// must be provided by the port binary.
    ///
    /// # Errors
    /// Will only fail on RNG failure.
    pub fn new(default_mac: [u8; 6], uart_pins: UartPins) -> Result<Self> {
        let hostkey = SignKey::generate(KeyType::Ed25519, None)?;

        // Wifi Access Point Mode
        let wifi_ap_ssid = Self::generate_wifi_ssid()?;
        let wifi_ap_pw = Self::generate_wifi_password()?;
        let wifi_ap_band = 0; // BandMode::Band2_4G (default)
        // Wifi Station Mode
        let wifi_sta_ssid = String::<32>::new();
        let wifi_sta_pw = String::<63>::new();
        let mac = default_mac;

        debug!(
            "SSH Stamp Config new() - RX Pin: {}  TX Pin: {}",
            uart_pins.rx, uart_pins.tx
        );

        Ok(SSHStampConfig {
            hostkey,
            pubkeys: Default::default(),
            wifi_ap_ssid,
            wifi_ap_pw,
            wifi_ap_band,
            wifi_sta_ssid,
            wifi_sta_pw,
            mac,
            ipv4_static: None,
            #[cfg(feature = "ipv6")]
            ipv6_static: None,
            uart_pins,
            uart_params: UartParams::default(),
            first_login: true,
        })
    }

    pub(crate) fn generate_wifi_ssid() -> Result<String<32>> {
        let mut rnd = [0u8; 16];
        getrandom::fill(&mut rnd).map_err(|_| sunset::Error::msg("RNG failed"))?;
        let mut ssid = String::<32>::new();
        for &byte in &rnd {
            let _ = ssid.push(WIFI_PASSWORD_CHARS[(byte as usize) % 62] as char);
        }
        Ok(ssid)
    }

    pub(crate) fn generate_wifi_password() -> Result<String<63>> {
        let mut rnd = [0u8; 24];
        getrandom::fill(&mut rnd).map_err(|_| sunset::Error::msg("RNG failed"))?;
        let mut pw = String::<63>::new();
        for &byte in &rnd {
            let _ = pw.push(WIFI_PASSWORD_CHARS[(byte as usize) % 62] as char);
        }
        Ok(pw)
    }

    // Password functions removed; pubkey-only auth supported.

    pub(crate) fn add_pubkey(&mut self, key_str: &str) -> Result<(), Error> {
        // Accept OpenSSH public key format (e.g. "ssh-ed25519 AAAA...") and
        // validate it is an Ed25519 key. Insert into the first empty slot or
        // overwrite slot 0 if none empty.

        debug!(
            "Checking pubkey string passed through ENV: {}",
            key_str.trim()
        );

        let openssh = PublicKey::from_str(key_str.trim())?;

        debug!("Public key format valid, continuing to parse");

        match openssh.key_data() {
            KeyData::Ed25519(k) => {
                let bytes = k.0; // [u8; 32]
                let newk = Ed25519PubKey { key: Blob(bytes) };

                debug!("Parsed Ed25519 public key, adding to config");
                for slot in &mut self.pubkeys {
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
    getrandom::fill(&mut mac).map_err(|_| sunset::Error::msg("RNG failed"))?;
    // unicast, locally administered
    mac[0] = (mac[0] & 0xfc) | 0x02;
    Ok(mac)
}

// a private encoding specific to demo config, not SSH defined.
fn enc_signkey(k: &SignKey, s: &mut dyn SSHSink) -> WireResult<()> {
    // need to add a variant field if we support more key types.
    match k {
        SignKey::Ed25519(k) => k.to_bytes().enc(s),
        SignKey::AgentEd25519(_) => Err(WireError::UnknownVariant),
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
pub(crate) fn enc_option<T: SSHEncode>(v: Option<&T>, s: &mut dyn SSHSink) -> WireResult<()> {
    v.is_some().enc(s)?;
    if let Some(v) = v {
        v.enc(s)?;
    }
    Ok(())
}

pub(crate) fn dec_option<'de, S, T: SSHDecode<'de>>(s: &mut S) -> WireResult<Option<T>>
where
    S: SSHSource<'de>,
{
    bool::dec(s)?.then(|| SSHDecode::dec(s)).transpose()
}

fn enc_ipv4_config(v: Option<&StaticConfigV4>, s: &mut dyn SSHSink) -> WireResult<()> {
    v.is_some().enc(s)?;
    if let Some(v) = v {
        v.address.address().to_bits().enc(s)?;
        debug!("enc_ipv4_config: prefix = {}", v.address.prefix_len());
        v.address.prefix_len().enc(s)?;
        // to u32
        let gw = v.gateway.as_ref().map(|g| g.to_bits());
        enc_option(gw.as_ref(), s)?;
    }
    Ok(())
}

#[cfg(feature = "ipv6")]
fn enc_ipv6_config(v: Option<&StaticConfigV6>, s: &mut dyn SSHSink) -> WireResult<()> {
    v.is_some().enc(s)?;
    if let Some(v) = v {
        v.address.address().octets().enc(s)?;
        v.address.prefix_len().enc(s)?;
        let gw = v.gateway.as_ref().map(core::net::Ipv6Addr::octets);
        enc_option(gw.as_ref(), s)?;
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
            // The embassy-net heapless version is different so `Default::default()` must be
            // used here.
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
        let ad: [u8; 16] = SSHDecode::dec(s)?;
        let ad = Ipv6Addr::from(ad);
        let prefix = SSHDecode::dec(s)?;
        if prefix > 128 {
            // embassy panics, so test it here. IPv6 prefixes run 0..=128;
            // this used to check the IPv4 bound of 32, rejecting every real
            // address (a /64 included) and tripping the recreate path in
            // `store::load_or_create`.
            return Err(WireError::PacketWrong);
        }
        let gw: Option<[u8; 16]> = dec_option(s)?;
        let gateway = gw.map(Ipv6Addr::from);
        Ok(StaticConfigV6 {
            address: Ipv6Cidr::new(ad, prefix),
            gateway,
            dns_servers: Default::default(),
        })
    })
    .transpose()
}

impl SSHEncode for SSHStampConfig {
    fn enc(&self, s: &mut dyn SSHSink) -> WireResult<()> {
        enc_signkey(&self.hostkey, s)?;

        for k in &self.pubkeys {
            enc_option(k.as_ref(), s)?;
        }

        // Wifi Access Point Mode
        self.wifi_ap_ssid.as_str().enc(s)?;
        self.wifi_ap_pw.as_str().enc(s)?;
        self.wifi_ap_band.enc(s)?;
        // Wifi Station Mode
        self.wifi_sta_ssid.as_str().enc(s)?;
        self.wifi_sta_pw.as_str().enc(s)?;
        self.mac.enc(s)?;

        enc_ipv4_config(self.ipv4_static.as_ref(), s)?;
        #[cfg(feature = "ipv6")]
        enc_ipv6_config(self.ipv6_static.as_ref(), s)?;

        // Encode UartPins
        self.uart_pins.rx.enc(s)?;
        self.uart_pins.tx.enc(s)?;

        // Encode UartParams
        self.uart_params.baud.enc(s)?;
        self.uart_params.data_bits.enc(s)?;
        (self.uart_params.parity as u8).enc(s)?;
        self.uart_params.stop_bits.enc(s)?;

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
        for k in &mut pubkeys {
            *k = dec_option(s)?;
        }

        // Wifi Access Point Mode
        let wifi_ap_ssid_str: &str = SSHDecode::dec(s)?;
        let wifi_ap_ssid = String::try_from(wifi_ap_ssid_str).map_err(|_| WireError::BadString)?;
        let wifi_ap_pw_str: &str = SSHDecode::dec(s)?;
        let wifi_ap_pw = String::try_from(wifi_ap_pw_str).map_err(|_| WireError::BadString)?;
        let wifi_ap_band: u8 = SSHDecode::dec(s)?;
        // Wifi Station Mode
        let wifi_sta_ssid_str: &str = SSHDecode::dec(s)?;
        let wifi_sta_ssid =
            String::try_from(wifi_sta_ssid_str).map_err(|_| WireError::BadString)?;
        let wifi_sta_pw_str: &str = SSHDecode::dec(s)?;
        let wifi_sta_pw = String::try_from(wifi_sta_pw_str).map_err(|_| WireError::BadString)?;

        let mac = SSHDecode::dec(s)?;

        let ipv4_static = dec_ipv4_config(s)?;
        #[cfg(feature = "ipv6")]
        let ipv6_static = dec_ipv6_config(s)?;

        // Not supported by sshwire-derive nor virtue (no Option<u8> support)
        // let uart_pins = SSHDecode::dec(s)?;
        let rx: u8 = SSHDecode::dec(s)?;
        let tx: u8 = SSHDecode::dec(s)?;
        let uart_pins = UartPins { rx, tx };

        // Decode UartParams
        let baud: u32 = SSHDecode::dec(s)?;
        let data_bits: u8 = SSHDecode::dec(s)?;
        let parity: u8 = SSHDecode::dec(s)?;
        let stop_bits: u8 = SSHDecode::dec(s)?;
        let uart_params = UartParams {
            baud,
            data_bits,
            parity: parity.into(),
            stop_bits,
        };

        let first_login = SSHDecode::dec(s)?;

        Ok(Self {
            hostkey,
            pubkeys,
            wifi_ap_ssid,
            wifi_ap_pw,
            wifi_ap_band,
            wifi_sta_ssid,
            wifi_sta_pw,
            mac,
            ipv4_static,
            #[cfg(feature = "ipv6")]
            ipv6_static,
            uart_pins,
            uart_params,
            first_login,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunset::sshwire;

    /// Drives the private `enc_ipv6_config`/`dec_ipv6_config` pair on its own,
    /// so a test does not have to hand-roll the rest of the config stream.
    #[cfg(feature = "ipv6")]
    struct Ipv6Slot(Option<StaticConfigV6>);

    #[cfg(feature = "ipv6")]
    impl SSHEncode for Ipv6Slot {
        fn enc(&self, s: &mut dyn SSHSink) -> WireResult<()> {
            enc_ipv6_config(self.0.as_ref(), s)
        }
    }

    #[cfg(feature = "ipv6")]
    impl<'de> SSHDecode<'de> for Ipv6Slot {
        fn dec<S>(s: &mut S) -> WireResult<Self>
        where
            S: SSHSource<'de>,
        {
            dec_ipv6_config(s).map(Ipv6Slot)
        }
    }

    /// `bool` tag + 16 address bytes + prefix + `bool` tag for an absent gateway.
    #[cfg(feature = "ipv6")]
    const IPV6_SLOT_NO_GW_LEN: usize = 1 + 16 + 1 + 1;
    #[cfg(feature = "ipv6")]
    const IPV6_PREFIX_OFFSET: usize = 17;

    #[cfg(feature = "ipv6")]
    fn ipv6_slot(prefix: u8) -> StaticConfigV6 {
        StaticConfigV6 {
            address: Ipv6Cidr::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), prefix),
            gateway: None,
            dns_servers: Default::default(),
        }
    }

    #[cfg(feature = "ipv6")]
    fn round_trip(cfg: &StaticConfigV6) -> Option<StaticConfigV6> {
        let mut buf = [0u8; 64];
        let n = sshwire::write_ssh(&mut buf, &Ipv6Slot(Some(cfg.clone()))).expect("encode");
        assert_eq!(n, IPV6_SLOT_NO_GW_LEN, "unexpected ipv6 slot wire layout");
        let (slot, _): (Ipv6Slot, usize) = sshwire::read_ssh(&buf[..n], None).expect("decode");
        slot.0
    }

    /// A /64 is the ordinary case. The decoder used to check the IPv4 bound of
    /// 32 here, so every realistic stored IPv6 config failed to decode — and a
    /// decode failure sends `store::load_or_create` down its recreate path.
    #[test]
    #[cfg(feature = "ipv6")]
    fn ipv6_prefix_64_round_trips() {
        let cfg = ipv6_slot(64);
        assert_eq!(round_trip(&cfg), Some(cfg));
    }

    /// 128 is the largest legal prefix and must be accepted, not rejected as
    /// an off-by-one.
    #[test]
    #[cfg(feature = "ipv6")]
    fn ipv6_prefix_128_round_trips() {
        let cfg = ipv6_slot(128);
        assert_eq!(round_trip(&cfg), Some(cfg));
    }

    #[test]
    #[cfg(feature = "ipv6")]
    fn ipv6_gateway_round_trips() {
        let cfg = StaticConfigV6 {
            address: Ipv6Cidr::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 64),
            gateway: Some(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)),
            dns_servers: Default::default(),
        };
        let mut buf = [0u8; 64];
        let n = sshwire::write_ssh(&mut buf, &Ipv6Slot(Some(cfg.clone()))).expect("encode");
        let (slot, _): (Ipv6Slot, usize) = sshwire::read_ssh(&buf[..n], None).expect("decode");
        assert_eq!(slot.0, Some(cfg));
    }

    /// Above 128 `Ipv6Cidr::new` asserts, so the decoder has to reject the
    /// bytes rather than hand them to smoltcp and panic the device on boot.
    #[test]
    #[cfg(feature = "ipv6")]
    fn ipv6_prefix_over_128_rejected() {
        let mut buf = [0u8; 64];
        let n = sshwire::write_ssh(&mut buf, &Ipv6Slot(Some(ipv6_slot(64)))).expect("encode");
        assert_eq!(n, IPV6_SLOT_NO_GW_LEN, "unexpected ipv6 slot wire layout");
        buf[IPV6_PREFIX_OFFSET] = 129;
        let decoded: Result<(Ipv6Slot, usize), _> = sshwire::read_ssh(&buf[..n], None);
        assert!(decoded.is_err(), "prefix 129 must not decode");
    }

    /// The IPv4 guard is the one the IPv6 guard was mistakenly copied from;
    /// pin its bound so a future edit cannot widen it to 128 as well.
    #[test]
    fn ipv4_prefix_over_32_rejected() {
        struct Ipv4Slot(Option<StaticConfigV4>);

        impl SSHEncode for Ipv4Slot {
            fn enc(&self, s: &mut dyn SSHSink) -> WireResult<()> {
                enc_ipv4_config(self.0.as_ref(), s)
            }
        }

        impl<'de> SSHDecode<'de> for Ipv4Slot {
            fn dec<S>(s: &mut S) -> WireResult<Self>
            where
                S: SSHSource<'de>,
            {
                dec_ipv4_config(s).map(Ipv4Slot)
            }
        }

        let cfg = StaticConfigV4 {
            address: Ipv4Cidr::new(Ipv4Addr::new(192, 168, 4, 1), 24),
            gateway: None,
            dns_servers: Default::default(),
        };
        let mut buf = [0u8; 64];
        let n = sshwire::write_ssh(&mut buf, &Ipv4Slot(Some(cfg))).expect("encode");
        // bool tag + 4 address bytes + prefix + bool tag for an absent gateway.
        assert_eq!(n, 1 + 4 + 1 + 1, "unexpected ipv4 slot wire layout");
        buf[5] = 33;
        let decoded: Result<(Ipv4Slot, usize), _> = sshwire::read_ssh(&buf[..n], None);
        assert!(decoded.is_err(), "prefix 33 must not decode");
    }
}
