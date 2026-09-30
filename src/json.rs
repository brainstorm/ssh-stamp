// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal JSON emission for machine-readable status output.
//!
//! ssh-stamp reports state in two places a program might want to read:
//! the serial console at boot (`WiFi` PSK, hostkey fingerprint) and the SSH
//! stderr channel during a session. Both were prose. This module provides
//! just enough to emit them as JSON instead, without pulling in a
//! serialiser — there is one shape per message and no need for reflection.
//!
//! # Conventions
//!
//! Every object is written on a single line and starts with the same key:
//!
//! ```text
//! {"ssh_stamp":1,"event":"boot",...}
//! ```
//!
//! [`VERSION`] doubles as a marker. A consumer can find ssh-stamp's output
//! amongst unrelated noise — the ESP32 ROM bootloader prelude, target UART
//! traffic — by looking for that prefix, without needing to know which
//! lines are ours. Bumping it signals an incompatible change; adding keys
//! does not, so consumers must ignore unknown ones.
//!
//! # Escaping
//!
//! Values reaching these messages include a user-set SSID, which can hold
//! any printable ASCII, quotes and backslashes included. [`Esc`] handles
//! that; using it for every string value is not optional, since one stray
//! quote turns a parseable document into a broken one.

use core::fmt::{Display, Formatter, Result, Write as _};
use core::net::Ipv4Addr;

use heapless::String;
use log::info;
use sunset::SignKey;

use crate::config::SSHStampConfig;
use crate::notices::band_label;

/// Schema version, and the marker identifying an ssh-stamp JSON line.
pub const VERSION: u32 = 1;

/// The opening of every ssh-stamp JSON object, including the event name.
///
/// Written as a prefix rather than composed from a map so the marker is
/// always first, which is what lets consumers match on a line prefix.
pub struct Head<'a>(pub &'a str);

impl Display for Head<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, r#"{{"ssh_stamp":{VERSION},"event":"{}""#, Esc(self.0))
    }
}

/// Escapes a string for use as a JSON string value, per RFC 8259.
///
/// Wraps the value only — callers supply the surrounding quotes, so this
/// composes inside a larger `write!`.
pub struct Esc<'a>(pub &'a str);

impl Display for Esc<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        for c in self.0.chars() {
            match c {
                '"' => f.write_str("\\\"")?,
                '\\' => f.write_str("\\\\")?,
                '\n' => f.write_str("\\n")?,
                '\r' => f.write_str("\\r")?,
                '\t' => f.write_str("\\t")?,
                // Everything below 0x20 must be escaped; \u is the only
                // form that covers the ones without a short escape.
                c if (c as u32) < 0x20 => write!(f, "\\u{:04x}", c as u32)?,
                c => f.write_char(c)?,
            }
        }
        Ok(())
    }
}

/// Prints the provisioning details as a single `boot` object.
///
/// The same facts as the `info!` lines logged at boot, in a form a script
/// can read. This is the only place they are available before a client can
/// connect: the WPA2 PSK is generated on first boot and printed nowhere
/// else.
///
/// Goes through `info!` like everything else, so the line carries the
/// logger's prefix; consumers strip it with the `grep -o` in `docs/USING.md`.
///
/// Secrets: unlike the SSH-side summary, the PSK *is* included. It has to
/// be — nobody can associate to the AP without it, and this is a local
/// serial cable, not a network peer.
///
/// The AP address is absent: it comes from the network stack, which is not
/// up yet. The port emits [`net_up`] once it is.
pub fn boot(config: &SSHStampConfig, mac: [u8; 6]) {
    let mut fp = String::<64>::new();
    if let SignKey::Ed25519(_) = config.hostkey
        && let Ok(f) = config.hostkey.pubkey().fingerprint()
    {
        let _ = write!(fp, "{f}");
    }

    info!(
        concat!(
            r#"{},"wifi_ap":{{"ssid":"{}","psk":"{}","band":"{}"}},"#,
            r#""mac":"{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}","#,
            r#""hostkey_fingerprint":"{}","first_login":{}}}"#
        ),
        Head("boot"),
        Esc(&config.wifi_ap_ssid),
        Esc(&config.wifi_ap_pw),
        band_label(config.wifi_ap_band),
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5],
        Esc(&fp),
        config.first_login,
    );
}

/// Prints the network details as a single `net_up` object, mirroring
/// [`boot`].
///
/// The address is only known once the stack is up, so it cannot be part of
/// `boot`. `role` distinguishes the device hosting its own AP (`"ap"`) from
/// it having joined one (`"station"`).
pub fn net_up(ssid: &str, role: &str, ip: Ipv4Addr) {
    info!(
        r#"{},"role":"{}","ssid":"{}","ip":"{}"}}"#,
        Head("net_up"),
        Esc(role),
        Esc(ssid),
        ip,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esc(s: &str) -> String<128> {
        let mut out = String::new();
        core::fmt::write(&mut out, format_args!("{}", Esc(s))).unwrap();
        out
    }

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(esc("ssh-stamp-a1b2").as_str(), "ssh-stamp-a1b2");
    }

    #[test]
    fn quotes_and_backslashes_are_escaped() {
        // A user-set SSID can contain both, and either one unescaped
        // breaks the whole document.
        assert_eq!(esc(r#"my "ssid""#).as_str(), r#"my \"ssid\""#);
        assert_eq!(esc(r"back\slash").as_str(), r"back\\slash");
    }

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(esc("a\nb\r\tc").as_str(), "a\\nb\\r\\tc");
        assert_eq!(esc("\x00\x1f").as_str(), "\\u0000\\u001f");
    }

    #[test]
    fn head_starts_with_the_version_marker() {
        let mut out = String::<128>::new();
        core::fmt::write(&mut out, format_args!("{}", Head("boot"))).unwrap();
        // Consumers match this prefix to pick our lines out of other
        // output, so its exact shape is load-bearing.
        assert_eq!(out.as_str(), r#"{"ssh_stamp":1,"event":"boot""#);
    }
}
