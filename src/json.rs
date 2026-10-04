// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal JSON emission for machine-readable status output.
//!
//! ssh-stamp reports its state on the serial console: the provisioning
//! details at boot (`WiFi` PSK, hostkey fingerprint), the network coming up,
//! stations joining, and its log. This module provides just enough to emit
//! that as JSON, without pulling in a serialiser — there is one shape per
//! message and no need for reflection.
//!
//! # Conventions
//!
//! Every object is built compact, starting with the same two keys, and
//! printed through [`Pretty`] so it is readable on a terminal:
//!
//! ```text
//! {
//!   "schema": 1,
//!   "event": "boot",
//!   ...
//! }
//! ```
//!
//! A top-level `{` and `}` are always alone on their lines, which is how a
//! consumer cuts our objects out of unrelated console noise such as the
//! ESP32 ROM bootloader prelude. [`SCHEMA`] is the format version: bumping
//! it signals an incompatible change; adding keys does not, so consumers
//! must ignore unknown ones.
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
use sunset::SignKey;

use crate::config::SSHStampConfig;
use crate::settings::SSH_STAMP_IDENT;

/// Version of the JSON format, emitted as `"schema"` on every object.
pub const SCHEMA: u32 = 1;

/// The opening of every ssh-stamp JSON object: the schema version, then the
/// event name.
pub struct Head<'a>(pub &'a str);

impl Display for Head<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, r#"{{"schema":{SCHEMA},"event":"{}""#, Esc(self.0))
    }
}

/// Escapes a value for use as a JSON string, per RFC 8259.
///
/// Takes anything `Display` and escapes it as it is written, so formatted
/// text needs no intermediate buffer. Wraps the value only — callers supply
/// the surrounding quotes, so this composes inside a larger `write!`.
pub struct Esc<T>(pub T);

impl<T: Display> Display for Esc<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(EscWriter(f), "{}", self.0)
    }
}

struct EscWriter<'a, W>(&'a mut W);

impl<W: core::fmt::Write> core::fmt::Write for EscWriter<'_, W> {
    fn write_str(&mut self, s: &str) -> Result {
        let f = &mut *self.0;
        for c in s.chars() {
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

/// Re-indents compact JSON for humans as it streams through.
///
/// Every object is still emitted compact and then reformatted here, so the
/// emitters stay simple and a top-level object always starts with `{` and
/// ends with `}` alone on their own lines, which is what lets a consumer cut
/// objects out of mixed console output. State is kept between calls to
/// [`feed`](Self::feed), since `write!` hands over an object in pieces.
#[derive(Default)]
struct PrettyState {
    depth: u8,
    in_str: bool,
    escaped: bool,
    /// Just opened a `{` or `[`; the newline waits in case it is empty.
    opened: bool,
}

impl PrettyState {
    /// Writes `s`, reformatted, to `w`.
    fn feed<W: core::fmt::Write>(&mut self, w: &mut W, s: &str) -> Result {
        for c in s.chars() {
            if self.in_str {
                w.write_char(c)?;
                if self.escaped {
                    self.escaped = false;
                } else if c == '\\' {
                    self.escaped = true;
                } else if c == '"' {
                    self.in_str = false;
                }
                continue;
            }
            if self.opened {
                self.opened = false;
                if matches!(c, '}' | ']') {
                    self.depth = self.depth.saturating_sub(1);
                    w.write_char(c)?;
                    continue;
                }
                self.newline(w)?;
            }
            match c {
                '{' | '[' => {
                    w.write_char(c)?;
                    self.depth = self.depth.saturating_add(1);
                    self.opened = true;
                }
                '}' | ']' => {
                    self.depth = self.depth.saturating_sub(1);
                    self.newline(w)?;
                    w.write_char(c)?;
                }
                ',' => {
                    w.write_char(',')?;
                    self.newline(w)?;
                }
                ':' => w.write_str(": ")?,
                '"' => {
                    self.in_str = true;
                    w.write_char('"')?;
                }
                c => w.write_char(c)?,
            }
        }
        Ok(())
    }

    fn newline<W: core::fmt::Write>(&self, w: &mut W) -> Result {
        w.write_char('\n')?;
        for _ in 0..self.depth {
            w.write_str("  ")?;
        }
        Ok(())
    }
}

/// Displays compact JSON pretty-printed; see [`PrettyState`].
pub struct Pretty<T>(T);

impl<T> Pretty<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }
}

impl<T: Display> Display for Pretty<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let mut w = PrettyWriter {
            out: f,
            state: PrettyState::default(),
        };
        write!(w, "{}", self.0)
    }
}

struct PrettyWriter<'a, W> {
    out: &'a mut W,
    state: PrettyState,
}

impl<W: core::fmt::Write> core::fmt::Write for PrettyWriter<'_, W> {
    fn write_str(&mut self, s: &str) -> Result {
        self.state.feed(self.out, s)
    }
}

/// A MAC address as lowercase colon-separated hex, e.g. `60:55:f9:f7:00:4c`.
pub struct Mac(pub [u8; 6]);

impl Display for Mac {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let m = self.0;
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            m[0], m[1], m[2], m[3], m[4], m[5]
        )
    }
}

/// Human name for the stored band code.
fn band_label(band: u8) -> &'static str {
    match band {
        0 => "2.4GHz",
        1 => "5GHz",
        2 => "auto",
        _ => "unknown",
    }
}

/// The provisioning details, as the `boot` object printed on the console.
///
/// Includes the PSK: it is generated on first boot and printed nowhere else,
/// and nobody can associate to the AP without it. The AP address is not
/// known yet; [`NetUp`] follows once the network stack has one.
pub struct Boot<'a> {
    pub config: &'a SSHStampConfig,
    pub mac: [u8; 6],
    pub channel: u8,
}

impl Display for Boot<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let c = self.config;
        let mut fp = String::<64>::new();
        if let SignKey::Ed25519(_) = c.hostkey
            && let Ok(k) = c.hostkey.pubkey().fingerprint()
        {
            let _ = write!(fp, "{k}");
        }
        write!(
            f,
            concat!(
                r#"{},"ident":"{}","#,
                r#""wifi_ap":{{"ssid":"{}","psk":"{}","band":"{}","channel":{}}},"#,
                r#""mac":"{}","hostkey_fingerprint":"{}","first_login":{}}}"#
            ),
            Head("boot"),
            Esc(SSH_STAMP_IDENT),
            Esc(&c.wifi_ap_ssid),
            Esc(&c.wifi_ap_pw),
            band_label(c.wifi_ap_band),
            self.channel,
            Mac(self.mac),
            Esc(&fp),
            c.first_login,
        )
    }
}

/// The network details, as the `net_up` object printed on the console.
///
/// `role` is `"ap"` when the device hosts its own AP, `"station"` when it
/// joined one.
pub struct NetUp<'a> {
    pub ssid: &'a str,
    pub role: &'a str,
    pub ip: Ipv4Addr,
}

impl Display for NetUp<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(
            f,
            r#"{},"role":"{}","ssid":"{}","ip":"{}"}}"#,
            Head("net_up"),
            Esc(self.role),
            Esc(self.ssid),
            self.ip,
        )
    }
}

/// A log record, as the `log` event the console logger prints.
pub struct Log<'a, M> {
    /// Lowercase level name, e.g. `"info"`.
    pub level: &'a str,
    pub target: &'a str,
    pub msg: M,
}

impl<M: Display> Display for Log<'_, M> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(
            f,
            r#"{},"level":"{}","target":"{}","msg":"{}"}}"#,
            Head("log"),
            Esc(self.level),
            Esc(self.target),
            Esc(&self.msg),
        )
    }
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
    fn head_starts_with_schema_and_event() {
        let mut out = String::<128>::new();
        core::fmt::write(&mut out, format_args!("{}", Head("boot"))).unwrap();
        assert_eq!(out.as_str(), r#"{"schema":1,"event":"boot""#);
    }

    fn pretty(s: &str) -> String<256> {
        let mut out = String::new();
        core::fmt::write(&mut out, format_args!("{}", Pretty::new(s))).unwrap();
        out
    }

    #[test]
    fn pretty_prints_one_field_per_line() {
        assert_eq!(
            pretty(r#"{"a":1,"b":{"c":"x"},"d":{}}"#).as_str(),
            "{\n  \"a\": 1,\n  \"b\": {\n    \"c\": \"x\"\n  },\n  \"d\": {}\n}"
        );
    }

    #[test]
    fn pretty_leaves_strings_alone_and_spans_fragments() {
        // Punctuation and escaped quotes inside a string are not structure.
        let compact = r#"{"s":"a,b:{c}\"d"}"#;
        let expected = "{\n  \"s\": \"a,b:{c}\\\"d\"\n}";
        assert_eq!(pretty(compact).as_str(), expected);

        // Split mid-string and mid-escape, as `write!` may hand it over.
        let mut st = PrettyState::default();
        let mut out = String::<256>::new();
        for frag in [r#"{"s":"a,b:{c}\"#, r#""d"}"#] {
            st.feed(&mut out, frag).unwrap();
        }
        assert_eq!(out.as_str(), expected);
    }
}
