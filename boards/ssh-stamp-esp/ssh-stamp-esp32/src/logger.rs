// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Console logger printing every record as a pretty-printed JSON `log` event.
//!
//! Installing a logger safely needs `log::set_logger`, which only exists on
//! targets with pointer-sized atomics. The ESP32-C2, C3 and S2 lack them, so
//! they keep esp-println's own logger and plain `LEVEL - message` lines.
//!
//! `@BENCH` records (target [`BENCH_TARGET`]) are printed raw: the benchmark
//! harness parses them as whitespace-separated `key=value` pairs.

#[cfg(target_has_atomic = "ptr")]
use log::{Level, LevelFilter, Log, Metadata, Record};
#[cfg(target_has_atomic = "ptr")]
use ssh_stamp::json;
#[cfg(target_has_atomic = "ptr")]
use ssh_stamp::mem_probe::BENCH_TARGET;

/// Installs the console logger, filtered by `ESP_LOG` at build time.
pub fn init() {
    #[cfg(target_has_atomic = "ptr")]
    if log::set_logger(&JsonLogger).is_ok() {
        log::set_max_level(max_level());
    }
    #[cfg(not(target_has_atomic = "ptr"))]
    esp_println::logger::init_logger_from_env();
}

/// `ESP_LOG`, e.g. `info` or `info,esp_radio=warn`.
#[cfg(target_has_atomic = "ptr")]
const FILTER: &str = match option_env!("ESP_LOG") {
    Some(f) => f,
    None => "info",
};

/// The level for `target`: the longest matching `prefix=level` directive,
/// else the bare `level` one, else `info`.
#[cfg(target_has_atomic = "ptr")]
fn level_for(target: &str) -> LevelFilter {
    let mut global = LevelFilter::Info;
    let mut best: Option<(usize, LevelFilter)> = None;
    for d in FILTER.split(',').map(str::trim) {
        match d.split_once('=') {
            Some((prefix, level)) => {
                if let Ok(level) = level.parse()
                    && target.starts_with(prefix)
                    && best.is_none_or(|(len, _)| prefix.len() > len)
                {
                    best = Some((prefix.len(), level));
                }
            }
            None => {
                if let Ok(level) = d.parse() {
                    global = level;
                }
            }
        }
    }
    best.map_or(global, |(_, level)| level)
}

/// The most verbose level any directive enables, so `log` can skip
/// formatting everything below it.
#[cfg(target_has_atomic = "ptr")]
fn max_level() -> LevelFilter {
    FILTER
        .split(',')
        .filter_map(|d| d.rsplit('=').next()?.trim().parse().ok())
        .max()
        .unwrap_or(LevelFilter::Info)
}

#[cfg(target_has_atomic = "ptr")]
struct JsonLogger;

#[cfg(target_has_atomic = "ptr")]
impl Log for JsonLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= level_for(metadata.target())
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if record.target() == BENCH_TARGET {
            esp_println::println!("{}", record.args());
            return;
        }
        let level = match record.level() {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
            Level::Trace => "trace",
        };
        esp_println::println!(
            "{}",
            json::Pretty::new(json::Log {
                level,
                target: record.target(),
                msg: record.args(),
            })
        );
    }

    fn flush(&self) {}
}
