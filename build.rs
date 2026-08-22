// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build configuration

use esp_config::{ConfigOption, Validator, Value, generate_config};

fn main() {
    emit_ssh_ident();

    generate_config(
        "ssh-stamp",
        &[option(
            "heap_size",
            "Global allocator heap size in bytes",
            60 * 1024,
            16 * 1024..257 * 1024,
        )],
        true,
        true,
    );
}

/// Emits `SSH_STAMP_IDENT` from the `sunset` version.
fn emit_ssh_ident() {
    println!("cargo:rerun-if-changed=Cargo.lock");
    let lock_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock");
    let lock = std::fs::read_to_string(&lock_path).unwrap();
    let sunset_ver = lock
        .split("[[package]]")
        .find(|s| s.contains("name = \"sunset\""))
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.trim()
                    .strip_prefix("version = ")
                    .map(|v| v.trim_matches('"'))
            })
        })
        .unwrap_or("unknown");
    let ident = format!(
        "SSH-2.0-Sunset-{sunset_ver}-ssh-stamp-{}",
        env!("CARGO_PKG_VERSION")
    );
    println!("cargo::rustc-env=SSH_STAMP_IDENT={ident}");
}

/// A byte integer option.
fn option(
    name: &str,
    description: &str,
    default: i128,
    range: std::ops::Range<i128>,
) -> ConfigOption {
    ConfigOption {
        name: name.to_string(),
        description: description.to_string(),
        default_value: Value::Integer(default),
        constraint: Some(Validator::IntegerInRange(range)),
        stability: esp_config::Stability::Unstable,
        active: true,
        display_hint: esp_config::DisplayHint::None,
    }
}
