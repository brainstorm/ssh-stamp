// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Parser for the firmware's `@BENCH key=value` sentinel lines.
//!
//! The firmware (with `mem-probe`/`crypto-bench`) prints lines like
//! `... @BENCH checkpoint=bench_boot t_us=412300`. A log prefix (timestamp,
//! level, module path from esp-println) may precede the sentinel, so we locate
//! `@BENCH ` anywhere in the line and parse everything after it as
//! whitespace-delimited `key=value` tokens. Values never contain spaces, so
//! this stays a trivial split.
//!
//! Lines are ANSI-stripped first. `esp-println` enables its `colors` feature by
//! default, so what actually reaches the wire is
//! `\x1b[32mINFO - @BENCH checkpoint=bench_boot t_us=412300\x1b[0m` — and since
//! every `@BENCH` line ends with its numeric field, the reset sequence lands on
//! that number. Unstripped, `t_us` parsed as `None`, which silently dropped
//! *every* checkpoint, KEX sample and heap high-water: the sentinel and the
//! string-valued fields survive colouring, so the run looked like firmware built
//! without `mem-probe` rather than like a parse failure.
//!
//! The format is deliberately kept hand-rolled: `defmt` or NDJSON via
//! `serde-json-core` would be the library answers, but `log`/`esp-println` is
//! already wired up on the device and the whole parser is a few dozen lines, so
//! neither pays for the churn.

use std::collections::BTreeMap;

/// The sentinel that marks a structured measurement line. Anything after it on
/// the same line is `key=value` pairs.
pub const SENTINEL: &str = "@BENCH ";

/// One parsed `@BENCH` record: an ordered map of key → raw string value.
/// Numeric accessors parse on demand so the parser never rejects a record over
/// a single malformed field.
#[derive(Debug, Clone, Default)]
pub struct Record {
    pub fields: BTreeMap<String, String>,
}

impl Record {
    /// Parses a single log line. Returns `None` if the line carries no
    /// `@BENCH ` sentinel or yields no `key=value` tokens. Tokens without an
    /// `=` are ignored.
    pub fn parse_line(line: &str) -> Option<Record> {
        // Cheap pre-filter on the raw line: colouring wraps the sentinel, it
        // never breaks it up, so a line without `@BENCH ` cannot gain one by
        // being stripped.
        if !line.contains(SENTINEL) {
            return None;
        }
        let plain = strip_ansi_escapes::strip_str(line);
        let idx = plain.find(SENTINEL)?;
        let rest = &plain[idx + SENTINEL.len()..];
        let mut fields = BTreeMap::new();
        for tok in rest.split_whitespace() {
            if let Some((k, v)) = tok.split_once('=') {
                fields.insert(k.to_string(), v.to_string());
            }
        }
        if fields.is_empty() {
            None
        } else {
            Some(Record { fields })
        }
    }

    /// Returns the raw string value for `key`, if present.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    /// Returns the value for `key` parsed as `u64`, if present and numeric.
    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    /// True if this record carries the given discriminator key (e.g.
    /// `"checkpoint"`, `"kex"`, `"heap"`, `"crypto"`).
    pub fn has(&self, key: &str) -> bool {
        self.fields.contains_key(key)
    }
}

/// Parses every `@BENCH` record out of a block of serial text (one per line).
pub fn parse_all<I, S>(lines: I) -> Vec<Record>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    lines
        .into_iter()
        .filter_map(|l| Record::parse_line(l.as_ref()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sentinel_after_log_prefix() {
        let r = Record::parse_line(
            "INFO  ssh_stamp::mem_probe: @BENCH kex=accept->firstauth elapsed_us=285700",
        )
        .unwrap();
        assert_eq!(r.get("kex"), Some("accept->firstauth"));
        assert_eq!(r.get_u64("elapsed_us"), Some(285700));
    }

    /// The real wire format: `esp-println`'s default `colors` feature wraps the
    /// whole log line, so the trailing reset is glued to the last value.
    #[test]
    fn parses_ansi_coloured_line() {
        let r = Record::parse_line(
            "\u{1b}[32mINFO - @BENCH checkpoint=bench_tcp_listening t_us=412300\u{1b}[0m",
        )
        .unwrap();
        assert_eq!(r.get("checkpoint"), Some("bench_tcp_listening"));
        assert_eq!(r.get_u64("t_us"), Some(412300));
    }

    /// The heap line's last value is glued to the ANSI reset; both it and
    /// mid-line keys like `max_bytes` (what `sweep --knob heap-size` bisects
    /// on) must still parse.
    #[test]
    fn parses_ansi_coloured_heap_line() {
        let r = Record::parse_line(
            "\u{1b}[32mINFO - @BENCH heap=wifi_up used_bytes=53900 total_bytes=73728 \
             max_bytes=61688 alloc_bytes=120000 freed_bytes=66100\u{1b}[0m",
        )
        .unwrap();
        assert_eq!(r.get("heap"), Some("wifi_up"));
        assert_eq!(r.get_u64("max_bytes"), Some(61688));
        assert_eq!(r.get_u64("freed_bytes"), Some(66100));
    }

    #[test]
    fn ignores_non_bench_lines() {
        assert!(Record::parse_line("HSM: accepting TCP on port 22").is_none());
    }

    #[test]
    fn ignores_tokens_without_equals() {
        let r = Record::parse_line("@BENCH crypto=sample op=mlkem_ek_parse stray i=3 cycles=6240")
            .unwrap();
        assert_eq!(r.get("crypto"), Some("sample"));
        assert_eq!(r.get("op"), Some("mlkem_ek_parse"));
        assert_eq!(r.get_u64("cycles"), Some(6240));
        assert!(!r.has("stray"));
    }
}
