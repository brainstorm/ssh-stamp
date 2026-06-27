// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Concatenates the upstream `sunset` SSH ident with the `ssh-stamp`
//! version, e.g. `SSH-2.0-Sunset-0.5.0-ssh-stamp-0.3.0`.

fn main() {
    println!("cargo:rerun-if-changed=Cargo.lock");
    let lock_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock");
    let lock = std::fs::read_to_string(&lock_path).unwrap();
    let sunset_ver = lock
        .split("[[package]]")
        .find(|s| s.contains("name = \"sunset\""))
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.trim().strip_prefix("version = ").map(|v| v.trim_matches('"')))
        })
        .unwrap_or("unknown");
    let ident = format!("SSH-2.0-Sunset-{sunset_ver}-ssh-stamp-{}", env!("CARGO_PKG_VERSION"));
    println!("cargo::rustc-env=SSH_STAMP_IDENT={ident}");
}