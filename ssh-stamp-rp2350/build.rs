// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Puts `memory.x` where the linker can find it.
//!
//! `link.x` (cortex-m-rt) and `link-rp.x` (embassy-rp, supplies the RP2350
//! `.start_block` image definition the boot ROM looks for) are pulled in via
//! rustflags in `.cargo/config.toml`.

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set by cargo"));
    File::create(out.join("memory.x"))
        .expect("cannot create memory.x in OUT_DIR")
        .write_all(include_bytes!("memory.x"))
        .expect("cannot write memory.x");

    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
}
