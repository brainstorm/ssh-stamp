// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Anthony Tambasco <anthony.tambasco@fastmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Hardware configuration types.

use core::net::Ipv6Addr;
use core::str::FromStr;
use heapless::String;

/// UART parity bit setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Parity {
    /// No parity bit (default).
    #[default]
    None,
    /// Even parity.
    Even,
    /// Odd parity.
    Odd,
}

impl FromStr for Parity {
    type Err = ();

    /// Parses a parity setting from a string value.
    ///
    /// Accepts `"none"`/`"n"`, `"even"`/`"e"` or `"odd"`/`"o"` (case-insensitive).
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("n") {
            Ok(Self::None)
        } else if value.eq_ignore_ascii_case("even") || value.eq_ignore_ascii_case("e") {
            Ok(Self::Even)
        } else if value.eq_ignore_ascii_case("odd") || value.eq_ignore_ascii_case("o") {
            Ok(Self::Odd)
        } else {
            Err(())
        }
    }
}

impl From<u8> for Parity {
    /// Resolves a `Parity` from its on-wire `u8` representation.
    ///
    /// Unknown values fall back to `None` (the default).
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Even,
            2 => Self::Odd,
            _ => Self::None,
        }
    }
}

/// UART line parameters for the SSH-to-serial bridge.
///
/// Persisted in the device config and applied when the bridge's UART is
/// brought up, so changes take effect on the next boot. Values are kept
/// target-agnostic; each port maps them onto its own UART driver types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UartParams {
    /// Baud rate in bits per second.
    pub baud: u32,
    /// Data bits per frame (5-8).
    pub data_bits: u8,
    /// Parity bit setting.
    pub parity: Parity,
    /// Stop bits per frame (1 or 2).
    pub stop_bits: u8,
}

impl Default for UartParams {
    /// The classic 115200 8N1.
    fn default() -> Self {
        Self {
            baud: 115_200,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
        }
    }
}

/// UART peripheral configuration.
///
/// Pin numbers (`tx_pin`, `rx_pin`) are target-specific and must be set by
/// the port binary before use. There are no cross-platform default values;
/// each port crate defines pin assignments in its `src/bin/` entry point.
/// See the `ssh-stamp-esp32` binary's module documentation for ESP32 defaults.
#[derive(Clone, Debug, Default)]
pub struct UartConfig {
    pub tx_pin: u8,
    pub rx_pin: u8,
    pub cts_pin: Option<u8>,
    pub rts_pin: Option<u8>,
    pub params: UartParams,
}

/// `WiFi` band mode for the access point.
///
/// Selects whether the AP operates on 2.4GHz, 5GHz, or both.
/// Only the ESP32-C5 supports 5GHz; other chips ignore the setting
/// and always operate on 2.4GHz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BandMode {
    /// 2.4 GHz only (default, supported by all ESP32 variants).
    #[default]
    Band2_4G,
    /// 5 GHz only (ESP32-C5 only).
    Band5G,
    /// Dual-band 2.4 GHz + 5 GHz (ESP32-C5 only).
    Auto,
}

impl FromStr for BandMode {
    type Err = ();

    /// Parses a `WiFi` band mode from a string value.
    ///
    /// Accepts `"2.4g"`, `"2g"`, `"24g"`, `"5g"`, or `"auto"` (case-insensitive).
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("2.4g")
            || value.eq_ignore_ascii_case("2g")
            || value.eq_ignore_ascii_case("24g")
        {
            Ok(Self::Band2_4G)
        } else if value.eq_ignore_ascii_case("5g") {
            Ok(Self::Band5G)
        } else if value.eq_ignore_ascii_case("auto") {
            Ok(Self::Auto)
        } else {
            Err(())
        }
    }
}

impl From<u8> for BandMode {
    /// Resolves a `BandMode` from its on-wire `u8` representation.
    ///
    /// Unknown values fall back to `Band2_4G` (the default).
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Band5G,
            2 => Self::Auto,
            _ => Self::Band2_4G,
        }
    }
}

/// Largest legal IPv6 prefix length.
pub const IPV6_MAX_PREFIX_LEN: u8 = 128;

/// Why an IPv6 configuration was refused.
///
/// Both variants describe a value that would panic rather than merely
/// misbehave if it reached the network stack, which is why they are rejected
/// at the edge instead of being clamped or ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ipv6ConfigError {
    /// The value is not one of `off`, `slaac`, or `<address>/<prefix>` with an
    /// optional `,<gateway>` suffix.
    Syntax,
    /// Prefix length above [`IPV6_MAX_PREFIX_LEN`]. `Ipv6Cidr::new` asserts on
    /// these.
    PrefixTooLong,
    /// An address that cannot sit on an interface or act as a gateway:
    /// multicast (which `Interface::update_ip_addrs` panics on), unspecified,
    /// or loopback.
    NotAssignable,
}

/// How the device obtains a routable IPv6 address.
///
/// Independent of the IPv4 setting: the stack runs both families at once, so
/// a station can hold a `DHCPv4` lease and a SLAAC address at the same time.
///
/// A link-local `fe80::` address derived from the MAC is present regardless of
/// this setting — `embassy-net` installs one on every config apply — so
/// `ssh -6 fe80::…%iface` reaches the device even when IPv6 is `Disabled`.
/// `Disabled` means "no routable address", not "no IPv6".
///
/// Deliberately built from `core::net` types only. An earlier revision stored
/// `embassy_net::StaticConfigV6` behind a cargo feature, which made the
/// on-flash layout depend on a build flag; flipping the flag across an OTA
/// silently failed the config's integrity check and wiped the host key. This
/// type compiles unconditionally, so there is one flash format for every build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ipv6Mode {
    /// No routable address is configured. Link-local only.
    #[default]
    Disabled,
    /// Stateless address autoconfiguration (RFC 4862): solicit a router and
    /// derive an address from the prefix it advertises.
    ///
    /// Station mode only in practice. Nothing on this device sends router
    /// advertisements, so a client attached to its access point has no router
    /// to hear from, and neither does the device itself.
    Slaac,
    /// A fixed address and prefix, with an optional default gateway.
    Static {
        address: Ipv6Addr,
        prefix_len: u8,
        gateway: Option<Ipv6Addr>,
    },
}

impl Ipv6Mode {
    /// Builds a validated [`Ipv6Mode::Static`].
    ///
    /// This is the only way to construct one, because both of the checks it
    /// makes guard against a panic rather than a wrong result: smoltcp's
    /// `Ipv6Cidr::new` asserts the prefix is at most 128, and
    /// `Interface::update_ip_addrs` panics outright on an address that is not
    /// unicast. Values reaching either of those come from flash or from an SSH
    /// environment variable, so they are checked here instead.
    ///
    /// # Errors
    /// [`Ipv6ConfigError::PrefixTooLong`] for a prefix over 128, or
    /// [`Ipv6ConfigError::NotAssignable`] for an address or gateway that is
    /// multicast, unspecified or loopback.
    pub fn new_static(
        address: Ipv6Addr,
        prefix_len: u8,
        gateway: Option<Ipv6Addr>,
    ) -> Result<Self, Ipv6ConfigError> {
        if prefix_len > IPV6_MAX_PREFIX_LEN {
            return Err(Ipv6ConfigError::PrefixTooLong);
        }
        if !is_assignable(&address) || gateway.is_some_and(|gw| !is_assignable(&gw)) {
            return Err(Ipv6ConfigError::NotAssignable);
        }
        Ok(Self::Static {
            address,
            prefix_len,
            gateway,
        })
    }
}

/// Whether an address can be installed on an interface or used as a gateway.
///
/// Multicast is what smoltcp panics on; the unspecified and loopback addresses
/// decode fine but are never a usable interface address, so they are refused
/// at the same gate rather than failing confusingly later.
fn is_assignable(addr: &Ipv6Addr) -> bool {
    !addr.is_multicast() && !addr.is_unspecified() && !addr.is_loopback()
}

impl FromStr for Ipv6Mode {
    type Err = Ipv6ConfigError;

    /// Parses the value of the `SSH_STAMP_IPV6` environment variable.
    ///
    /// Accepts `"off"`/`"none"`/`"disabled"`, `"slaac"`/`"auto"`, or an address
    /// in `<address>/<prefix>` form with an optional `,<gateway>` suffix —
    /// for example `"2001:db8::2/64,2001:db8::1"`. Comma rather than a space
    /// so the value survives `env_sanitize`, which requires printable
    /// non-space ASCII.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("off")
            || value.eq_ignore_ascii_case("none")
            || value.eq_ignore_ascii_case("disabled")
        {
            return Ok(Self::Disabled);
        }
        if value.eq_ignore_ascii_case("slaac") || value.eq_ignore_ascii_case("auto") {
            return Ok(Self::Slaac);
        }

        let (cidr, gateway) = match value.split_once(',') {
            Some((cidr, gw)) => (
                cidr,
                Some(
                    gw.trim()
                        .parse::<Ipv6Addr>()
                        .map_err(|_| Ipv6ConfigError::Syntax)?,
                ),
            ),
            None => (value, None),
        };
        let (address, prefix_len) = cidr.trim().split_once('/').ok_or(Ipv6ConfigError::Syntax)?;
        let address = address
            .trim()
            .parse::<Ipv6Addr>()
            .map_err(|_| Ipv6ConfigError::Syntax)?;
        let prefix_len = prefix_len
            .trim()
            .parse::<u8>()
            .map_err(|_| Ipv6ConfigError::Syntax)?;

        Self::new_static(address, prefix_len, gateway)
    }
}

/// `WiFi` access point configuration.
///
/// Contains settings for running the device as a `WiFi` access point.
#[derive(Clone, Debug)]
pub struct WifiApConfigStatic {
    /// Wifi Mode - Access Point (ap) or Station (sta) Mode. Access Point by default.
    /// Network name (SSID), max 32 characters.
    pub ap_ssid: String<32>,
    pub sta_ssid: String<32>,
    /// Mandatory `WiFi` password, max 63 characters.
    /// We don't want None here as it would present an open network,
    /// which is not something we want to support.
    pub ap_password: String<63>,
    pub sta_password: String<63>,
    /// `WiFi` channel (1-14 for 2.4GHz, 36+ for 5GHz).
    pub channel: u8,
    /// `WiFi` band mode (2.4GHz / 5GHz / Auto). Ignored on chips without 5GHz.
    pub band: BandMode,
    /// MAC address for the access point interface.
    pub mac: [u8; 6],
    /// How to configure IPv6 on the stack once it is up.
    pub ipv6: Ipv6Mode,
}

impl Default for WifiApConfigStatic {
    fn default() -> Self {
        Self {
            ap_ssid: String::new(),
            ap_password: String::new(),
            sta_ssid: String::new(),
            sta_password: String::new(),
            channel: 1,
            band: BandMode::default(),
            mac: [0; 6],
            ipv6: Ipv6Mode::default(),
        }
    }
}
