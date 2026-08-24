// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! This is the forwarding cargo command wrapper. Any cargo command can be forwarded
//! from the xtask in order to target a specific board or chip:
//! `cargo xtask <target> <cargo command> [args...]`.

use crate::board::Target;
use crate::cmd::shell;
use anyhow::{Context, Result, bail};
use xshell::cmd;

/// Runs `argv` as `[target, cargo command, args...]`.
pub fn run(argv: &[String]) -> Result<()> {
    let name = argv.first().context("missing a target name")?;
    let Some(target) = Target::find(name) else {
        bail!("`{name}` is not a known xtask command, see `cargo xtask --help`");
    };

    let Some(action) = argv.get(1) else {
        bail!("expected a cargo command after `{name}`");
    };
    if action == "run" && matches!(target, Target::Chip(_)) {
        bail!("`{name}` is a library only chip, `run` needs a board, see `cargo xtask list`");
    }

    let sh = shell()?;
    let toolchain = format!("+{}", target.toolchain());
    let selection = target.selection_args();
    let trailing = &argv[2..];

    eprintln!("=== {action} {} ===", target.name());

    cmd!(
        sh,
        "cargo {toolchain} {action} {selection...} {trailing...}"
    )
    .env("CARGO_TARGET_DIR", target.target_dir())
    .run()
    .with_context(|| format!("cargo {action} for {} failed", target.name()))
}
