// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

fn main() {
    println!("cargo:rustc-link-arg-bins=-Tlinkall.x");
}
