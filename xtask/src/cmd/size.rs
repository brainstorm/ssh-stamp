// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask size` determines the flash and RAM size of the firmware.

use crate::board::{self, Board};
use crate::elf;
use crate::results::{Results, SizeResults, SocSize};
use crate::stats::fmt_bytes;
use anyhow::{Result, bail};
use clap::{ArgGroup, Args as ClapArgs};
use std::path::PathBuf;
use xshell::Shell;
use crate::elf::Footprint;

#[derive(ClapArgs)]
#[command(group(ArgGroup::new("selection").required(true)))]
pub struct Args {
    /// Boards to measure, use `--all` for every board.
    #[arg(long = "board", value_parser = board::name_parser(), group = "selection")]
    pub(crate) boards: Vec<&'static Board>,
    /// Measure every supported board.
    #[arg(long, group = "selection")]
    pub(crate) all: bool,
    /// How many crates to list for `cargo bloat`.
    #[arg(long, default_value_t = 20)]
    top: u32,
    /// Write results JSON here.
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
}

pub fn run(args: &Args) -> Result<()> {
    let boards = board::select(&args.boards, args.all);
    let sh = Shell::new()?;

    let mut violations: Vec<String> = Vec::new();

    let mut entries = Vec::new();
    for board in boards {
        let profile = board::PROFILE;
        let features = board.features(&[]);

        board.build(&sh, profile, &features, &[])?;

        let footprint = Footprint::new(&board.elf_path(profile), board)?;
        let crates = board
            .bloat(&sh, profile, &features, args.top)
            .unwrap_or_default();

        for (label, value, cap) in [
            ("flash", footprint.flash_b, board.max_flash_kib),
            ("RAM", footprint.ram_b, board.max_ram_kib),
        ] {
            if let Some(cap) = cap
                && value > cap * 1024
            {
                violations.push(format!(
                    "{}: {label} {} exceeds cap {}",
                    board.name,
                    fmt_bytes(value),
                    fmt_bytes(cap * 1024)
                ));
            }
        }

        let entry = SocSize {
            soc: board.soc.to_string(),
            board: board.name.to_string(),
            profile: profile.to_string(),
            target: board.target.to_string(),
            features,
            flash_bytes: footprint.flash_b,
            ram_bytes: footprint.ram_b,
            stack_reserved_bytes: footprint.stack_reserved_b,
            crates,
        };

        print_entry(&entry);
        entries.push(entry);
    }

    if let Some(out) = &args.output {
        Results::Size(SizeResults { entries }).write(out)?;
    }

    if !violations.is_empty() {
        eprintln!("\n=== size FAILED ===");
        for violation in &violations {
            eprintln!("  ✗ {violation}");
        }
        bail!("{} size check/s failed", violations.len());
    }

    Ok(())
}

fn print_entry(e: &SocSize) {
    println!("\n=== {} [{}] ===", e.board, e.profile);
    println!("  soc      : {}", e.soc);
    println!("  target   : {}", e.target);
    println!("  features : {}", e.features);
    println!("  flash    : {:>10}", fmt_bytes(e.flash_bytes));
    println!("  RAM      : {:>10}", fmt_bytes(e.ram_bytes));
    println!("  stack    : {:>10}", fmt_bytes(e.stack_reserved_bytes));
    if !e.crates.is_empty() {
        println!("  top crates by .text:");
        for c in e.crates.iter().take(10) {
            println!("    {:>10}  {}", fmt_bytes(c.size_bytes), c.name);
        }
    }
}
