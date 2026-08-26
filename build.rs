// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build configuration

fn main() {
    emit_ssh_ident();
    emit_config_offset();
}

/// Where the configuration area lives in flash.
///
/// A property of the port's flash layout, not of ssh-stamp: 0x9000 is where an
/// ESP-IDF partition table puts NVS, and it means nothing on a part that does
/// not use one. Emitted as a compile-time constant rather than passed at
/// runtime because `store`'s tests use it to size arrays, and it follows the
/// same shape as `SSH_STAMP_IDENT` below.
///
/// Override with `SSH_STAMP_CONFIG_OFFSET`, decimal or `0x`-prefixed hex.
fn emit_config_offset() {
    const DEFAULT: usize = 0x9000;
    println!("cargo:rerun-if-env-changed=SSH_STAMP_CONFIG_OFFSET");

    let offset = match std::env::var("SSH_STAMP_CONFIG_OFFSET") {
        Ok(v) => {
            let t = v.trim().to_owned();
            let parsed = t
                .strip_prefix("0x")
                .or_else(|| t.strip_prefix("0X"))
                .map_or_else(|| t.parse::<usize>(), |h| usize::from_str_radix(h, 16));
            parsed.unwrap_or_else(|_| panic!("SSH_STAMP_CONFIG_OFFSET is not a number: {v:?}"))
        }
        Err(_) => DEFAULT,
    };
    println!("cargo::rustc-env=SSH_STAMP_CONFIG_OFFSET={offset}");
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
