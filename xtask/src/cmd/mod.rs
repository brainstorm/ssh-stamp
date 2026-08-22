// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The subcommand implementations.

use anyhow::Result;
use xshell::Shell;

pub mod bench;
pub mod bmf;
pub mod cargo;
pub mod list;
pub mod size;

/// A shell with the workspace as the root, so xtask works from any directory.
pub fn shell() -> Result<Shell> {
    let sh = Shell::new()?;
    sh.change_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    Ok(sh)
}

/// The extra features every build has.
pub const BENCH_FEATURES: &[&str] = &["mem-probe", "bench-loopback"];

/// The default KEX for measurements not about key exchange.
pub const REFERENCE_KEX: &str = "mlkem768x25519-sha256";

/// The boot `@BENCH checkpoint=` name of the firmware.
pub const BOOT: &str = "bench_boot";
/// The peripherals ready `@BENCH checkpoint=` name of the firmware.
pub const PERIPHERALS_READY: &str = "bench_peripherals_ready";
/// The Wi-Fi up `@BENCH checkpoint=` name of the firmware.
pub const WIFI_UP: &str = "bench_wifi_up";
/// The TCP listening `@BENCH checkpoint=` name of the firmware.
pub const TCP_LISTENING: &str = "bench_tcp_listening";

/// The startup checkpoints in the order.
pub const BOOT_CHECKPOINTS: &[&str] = &[BOOT, PERIPHERALS_READY, WIFI_UP, TCP_LISTENING];

/// True if `name` is a boot checkpoint.
pub fn is_boot_checkpoint(name: &str) -> bool {
    BOOT_CHECKPOINTS.contains(&name)
}
