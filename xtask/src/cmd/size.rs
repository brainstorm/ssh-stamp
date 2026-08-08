// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask size` — per-SoC flash / RAM budget and the size gate (milestone (c)).
//!
//! For each selected SoC this builds the shipping firmware, reads the linked
//! ELF's section sizes (flash = `.text` + `.rodata`, RAM = `.data` + `.bss`),
//! and — as a supplement — attributes `.text` to crates with `cargo bloat
//! --crates`. It emits one JSON document so the report renderer and Bencher can
//! ingest it.
//!
//! It is also the **gate**, failing the run on either of:
//!
//! * a blown `--max-flash-kib` / `--max-ram-kib` cap, or
//! * a missing overflow-checks enforcement in `.cargo/config.toml` — milestone
//!   (c) is to minimise size *without* trading away safety, so removing that
//!   rustflag must not pass CI. See [`crate::safety`].

use crate::elf;
use crate::results::{Results, SizeResults, SocSize};
use crate::safety;
use crate::soc;
use crate::stats::fmt_b;
use anyhow::{Result, bail};
use clap::Args as ClapArgs;
use std::path::PathBuf;
use xshell::Shell;

#[derive(ClapArgs)]
pub struct Args {
    /// SoC to measure (repeatable). Defaults to `esp32c6`; use `--all` for the
    /// whole family.
    #[arg(long = "soc")]
    socs: Vec<String>,
    /// Measure every supported SoC.
    #[arg(long)]
    all: bool,
    /// Cargo profile override (defaults to each SoC's shipping profile:
    /// `release`, or `esp32s2` for the S2). Use `release-min` for the
    /// size-minimised profile.
    #[arg(long)]
    profile: Option<String>,
    /// Extra firmware features on top of the shipping set, comma-separated
    /// (e.g. `mem-probe`).
    #[arg(long)]
    features: Option<String>,
    /// How many crates to list in the `cargo bloat` breakdown.
    #[arg(long, default_value_t = 20)]
    top: u32,
    /// Skip `cargo build` (assume the ELF is already current).
    #[arg(long)]
    no_build: bool,
    /// Fail if any SoC's flash (`.text`+`.rodata`) exceeds this many KiB.
    #[arg(long)]
    max_flash_kib: Option<u64>,
    /// Fail if any SoC's static RAM (`.data`+`.bss`) exceeds this many KiB.
    #[arg(long)]
    max_ram_kib: Option<u64>,
    /// Cargo config to verify the overflow-checks enforcement in.
    #[arg(long, default_value = ".cargo/config.toml")]
    cargo_config: PathBuf,
    /// Write results JSON here.
    #[arg(long)]
    json: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<()> {
    let socs = soc::select(&args.socs, args.all)?;
    let sh = Shell::new()?;
    let extra: Vec<&str> = args
        .features
        .as_deref()
        .map(|f| f.split(',').filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    let mut violations: Vec<String> = Vec::new();

    // Safety gate first: it is a config-level check, so it fires before we spend
    // time building anything.
    let enforced_by = safety::enforcing_targets(&args.cargo_config)?;
    if enforced_by.is_empty() {
        violations.push(format!(
            "no [target.*] table in {} applies `-C overflow-checks=on` — size must not \
             drop the overflow checks (milestone (c))",
            args.cargo_config.display()
        ));
    }

    let mut entries = Vec::new();
    for soc in socs {
        let profile = args.profile.as_deref().unwrap_or(soc.default_profile);
        let features = soc.features(&extra);

        if !args.no_build {
            soc.build(&sh, profile, &features)?;
        }

        let sections = elf::sections(&soc.elf_path(profile))?;
        let flash_b = elf::sum_sections(&sections, &[".text", ".rodata"]);
        let ram_b = elf::sum_sections(&sections, &[".data", ".bss"]);
        let crates = soc
            .bloat(&sh, profile, &features, args.top)
            .unwrap_or_default();

        for (label, value, cap) in [
            ("flash", flash_b, args.max_flash_kib),
            ("RAM", ram_b, args.max_ram_kib),
        ] {
            if let Some(cap) = cap
                && value > cap * 1024
            {
                violations.push(format!(
                    "{}: {label} {} exceeds cap {}",
                    soc.name,
                    fmt_b(value as f64),
                    fmt_b((cap * 1024) as f64)
                ));
            }
        }

        let entry = SocSize {
            soc: soc.name.to_string(),
            profile: profile.to_string(),
            target: soc.target.to_string(),
            features,
            flash_b,
            ram_b,
            sections,
            crates,
        };
        print_entry(&entry);
        entries.push(entry);
    }

    if enforced_by.is_empty() {
        eprintln!("\noverflow-checks: NOT ENFORCED");
    } else {
        eprintln!(
            "\noverflow-checks: enforced by {}",
            enforced_by
                .iter()
                .map(|k| format!("[target.{k}]"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if let Some(json) = &args.json {
        Results::Size(SizeResults {
            entries,
            overflow_checks_enforced_by: enforced_by,
        })
        .write(json)?;
    }

    if !violations.is_empty() {
        eprintln!("\n=== size gate FAILED ===");
        for v in &violations {
            eprintln!("  ✗ {v}");
        }
        bail!("{} size/safety check(s) failed", violations.len());
    }
    Ok(())
}

fn print_entry(e: &SocSize) {
    println!("\n=== {} [{}] ===", e.soc, e.profile);
    println!("  target   : {}", e.target);
    println!("  features : {}", e.features);
    println!(
        "  flash    : {:>10}  (.text + .rodata)",
        fmt_b(e.flash_b as f64)
    );
    println!("  RAM      : {:>10}  (.data + .bss)", fmt_b(e.ram_b as f64));
    if !e.crates.is_empty() {
        println!("  top crates by .text:");
        for c in e.crates.iter().take(10) {
            println!("    {:>10}  {}", fmt_b(c.size_b as f64), c.name);
        }
    }
}
