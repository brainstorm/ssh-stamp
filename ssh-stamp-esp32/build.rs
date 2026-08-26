// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Gabriel Ku Wei Bin <gabriel.ku@fsfe.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use esp_config::{ConfigOption, Validator, Value, generate_config};

fn main() {
    println!("cargo:rustc-link-arg-bins=-Tlinkall.x");
    // esp-radio sets this cfg on 5GHz-capable chips (ESP32-C5).
    println!("cargo:rustc-check-cfg=cfg(wifi_has_5g)");

    // The heap size belongs to whoever installs the allocator, which is this
    // crate. It used to be declared by the platform-agnostic `ssh-stamp`
    // crate, which forced an `esp-config` dependency onto code that has no
    // business knowing about Espressif — and meant that crate could not be
    // built for a non-Espressif target at all.
    //
    // The "ssh-stamp" prefix is deliberate: it keeps the environment variable
    // spelled `SSH_STAMP_CONFIG_HEAP_SIZE`, which `xtask bench` sweeps by name.
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
