// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The parser for the `@BENCH key=value` lines used by the benchmarking firmware.

use std::collections::BTreeMap;
use strip_ansi_escapes::strip_str;

/// The name of the benchmarking line marker.
pub const BENCH_MARKER: &str = "@BENCH ";

/// Represents a single `@BENCH` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// A checkpoint in the boot or session code path.
    Checkpoint { name: String, t_us: u64 },
    /// The KEX time.
    Kex { elapsed_us: u64 },
    /// The heap usage at a given labelled point.
    Heap {
        label: String,
        used_bytes: u64,
        total_bytes: u64,
        max_bytes: u64,
    },
}

impl Record {
    /// Parse a log line and return the representing record.
    pub fn parse_line(line: &str) -> Option<Record> {
        if !line.contains(BENCH_MARKER) {
            return None;
        }

        let line = strip_str(line);
        let fields: BTreeMap<_, _> = line
            .split_once(BENCH_MARKER)?
            .1
            .split_whitespace()
            .filter_map(|tok| tok.split_once('='))
            .collect();
        let parse_number = |key: &str| fields.get(key).and_then(|v| v.parse::<u64>().ok());

        if let Some(name) = fields.get("checkpoint") {
            return Some(Record::Checkpoint {
                name: name.to_string(),
                t_us: parse_number("t_us")?,
            });
        }

        if fields.contains_key("kex") {
            return Some(Record::Kex {
                elapsed_us: parse_number("elapsed_us")?,
            });
        }

        if let Some(label) = fields.get("heap") {
            return Some(Record::Heap {
                label: label.to_string(),
                used_bytes: parse_number("used_bytes").unwrap_or(0),
                total_bytes: parse_number("total_bytes").unwrap_or(0),
                max_bytes: parse_number("max_bytes").unwrap_or(0),
            });
        }

        None
    }
}

/// Parses all `@BENCH` records out of the lines.
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
    fn parses_from_log() {
        assert_eq!(
            Record::parse_line(
                "INFO  ssh_stamp::mem_probe: @BENCH kex=accept->firstauth elapsed_us=285700"
            ),
            Some(Record::Kex {
                elapsed_us: 285_700
            })
        );

        assert_eq!(
            Record::parse_line(
                "\u{1b}[32mINFO - @BENCH checkpoint=bench_tcp_listening t_us=412300\u{1b}[0m"
            ),
            Some(Record::Checkpoint {
                name: "bench_tcp_listening".into(),
                t_us: 412_300
            })
        );

        assert_eq!(
            Record::parse_line(
                "\u{1b}[32mINFO - @BENCH heap=wifi_up used_bytes=53900 total_bytes=73728 \
                 max_bytes=61688 alloc_bytes=120000 freed_bytes=66100\u{1b}[0m"
            ),
            Some(Record::Heap {
                label: "wifi_up".into(),
                used_bytes: 53_900,
                total_bytes: 73_728,
                max_bytes: 61_688,
            })
        );
    }

    #[test]
    fn ignore_not_parsable_lines() {
        assert!(Record::parse_line("HSM: accepting TCP on port 22").is_none());
        assert!(Record::parse_line("@BENCH something=else").is_none());
    }

    #[test]
    fn ignore_without_equals() {
        assert_eq!(
            Record::parse_line("@BENCH heap=wifi_up stray used_bytes=53900 max_bytes=54000"),
            Some(Record::Heap {
                label: "wifi_up".into(),
                used_bytes: 53_900,
                total_bytes: 0,
                max_bytes: 54_000,
            })
        );
    }

    #[test]
    fn remove_unusable_measurement() {
        assert!(Record::parse_line("@BENCH checkpoint=bench_boot t_us=NaN").is_none());
        assert!(Record::parse_line("@BENCH checkpoint=bench_boot").is_none());
        assert!(Record::parse_line("@BENCH kex=accept->firstauth").is_none());
        assert_eq!(
            Record::parse_line("@BENCH heap=boot max_bytes=1000"),
            Some(Record::Heap {
                label: "boot".into(),
                used_bytes: 0,
                total_bytes: 0,
                max_bytes: 1000,
            })
        );
    }
}
