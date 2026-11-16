// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask test`
//!
//! Test suites that need more than `cargo test`, such as real hardware.

use anyhow::Result;
use clap::{Args as ClapArgs, Subcommand};

pub mod e2e;

#[derive(ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run end-to-end integration tests on hardware.
    E2e(e2e::Args),
}

pub fn run(args: &Args) -> Result<()> {
    match &args.command {
        Command::E2e(args) => e2e::run(args),
    }
}
