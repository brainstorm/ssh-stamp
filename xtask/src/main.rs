// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! ssh-stamp benchmark & automation runner. See `xtask/README.md`.
//!
//! A single cross-platform task runner. It drives `cargo`/`espflash`/`ssh` as
//! subprocesses, reads the device's `@BENCH key=value` serial lines, reads the
//! linked ELF in-process, and emits `results.json` / `bench-report.md` / Bencher
//! Metric Format.

mod cmd;
mod device;
mod elf;
mod record;
mod results;
mod safety;
mod soc;
mod stats;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "xtask",
    about = "ssh-stamp benchmark & automation runner",
    version,
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build, flash, drive SSH sessions and capture the boot→session timeline
    /// plus KEX wall times (`--compare` for the mlkem A/B).
    Bench(cmd::bench::Args),
    /// Render bench-report.md from collected results.json.
    Report(cmd::report::Args),
    /// Convert collected results.json into Bencher Metric Format for CI tracking.
    Bmf(cmd::bmf::Args),
    /// Build, flash and capture the boot-time crypto microbench; statistics
    /// are computed host-side from the raw per-iteration samples.
    Crypto(cmd::crypto::Args),
    /// Per-SoC flash/RAM budget with a hard cap + overflow-checks safety gate.
    Size(cmd::size::Args),
    /// Static per-function stack frames from a dedicated emit-stack-sizes build.
    Stack(cmd::stack::Args),
    /// Sweep one heap/buffer knob under load and report the smallest healthy
    /// value.
    Sweep(cmd::sweep::Args),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Bench(args) => cmd::bench::run(args),
        Command::Report(args) => cmd::report::run(args),
        Command::Bmf(args) => cmd::bmf::run(args),
        Command::Crypto(args) => cmd::crypto::run(args),
        Command::Size(args) => cmd::size::run(args),
        Command::Stack(args) => cmd::stack::run(args),
        Command::Sweep(args) => cmd::sweep::run(args),
    }
}
