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

#[cfg(test)]
mod tests {
    use super::*;
    use heapless::String;

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
