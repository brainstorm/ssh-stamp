// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

fn main() {
    // Based on the embedded-test examples and documentation.
    // See: <https://crates.io/crates/embedded-test>
    println!("cargo::rustc-check-cfg=cfg(rust_analyzer)");
    println!("cargo:rustc-link-arg=-Tembedded-test.x");
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}
