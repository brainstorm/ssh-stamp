// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! List known boards and chips.

use crate::board;
use anyhow::Result;
use clap::Args as ClapArgs;
use serde::Serialize;

#[derive(ClapArgs)]
pub struct Args {
    /// Emit the table as JSON, to drive a GitHub Actions build matrix.
    #[arg(long)]
    json: bool,
}

/// A board, with the fields a release build matrix needs.
#[derive(Serialize)]
struct BoardEntry {
    /// The board name, which is also the `cargo xtask <target>` argument.
    name: &'static str,
    /// The `SoC`, which is what `espflash --chip` takes.
    soc: &'static str,
    /// The rust target, one component of the build output path.
    triple: &'static str,
    /// Whether the board needs the xtensa toolchain instead of stable.
    xtensa: bool,
    /// The partition table to flash with, empty when the board declares none.
    partitions: &'static str,
}

/// A chip, which builds the library only and so has no image to flash.
#[derive(Serialize)]
struct ChipEntry {
    /// The chip name, which is also the `cargo xtask <target>` argument.
    name: &'static str,
    /// The rust target, one component of the build output path.
    triple: &'static str,
    /// Whether the chip needs the xtensa toolchain instead of stable.
    xtensa: bool,
}

/// Both tables, so a single call can feed any workflow's matrix.
#[derive(Serialize)]
struct Targets {
    /// The boards that build a full firmware binary.
    boards: Vec<BoardEntry>,
    /// The chips that build the library only.
    chips: Vec<ChipEntry>,
}

/// The toolchain name that marks a target as needing the xtensa toolchain.
const XTENSA_TOOLCHAIN: &str = "esp";

impl Targets {
    /// Collect the board and chip tables.
    fn collect() -> Self {
        Self {
            boards: board::BOARDS
                .iter()
                .map(|board| BoardEntry {
                    name: board.name,
                    soc: board.soc,
                    triple: board.target,
                    xtensa: !board.riscv,
                    // An empty string rather than null: the workflow tests this
                    // with `[ -n ... ]` after expression interpolation.
                    partitions: board.partitions.unwrap_or_default(),
                })
                .collect(),
            chips: board::CHIPS
                .iter()
                .map(|chip| ChipEntry {
                    name: chip.name,
                    triple: chip.target,
                    xtensa: chip.toolchain == XTENSA_TOOLCHAIN,
                })
                .collect(),
        }
    }
}

pub fn run(args: &Args) -> Result<()> {
    if args.json {
        println!("{}", serde_json::to_string(&Targets::collect())?);
        return Ok(());
    }

    println!("Boards (firmware binary):");
    for board in board::BOARDS {
        println!(
            "    {:<38} {}  (+{})",
            board.name, board.target, board.toolchain
        );
    }

    println!();
    println!("Chips (library-only build):");
    for chip in board::CHIPS {
        println!(
            "    {:<38} {}  (+{})",
            chip.name, chip.target, chip.toolchain
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_covers_every_target() {
        let targets = Targets::collect();

        assert_eq!(targets.boards.len(), board::BOARDS.len());
        assert_eq!(targets.chips.len(), board::CHIPS.len());

        for (entry, board) in targets.boards.iter().zip(board::BOARDS) {
            assert_eq!(entry.name, board.name);
            assert_eq!(entry.soc, board.soc);
            assert_eq!(entry.triple, board.target);
            assert_eq!(entry.xtensa, board.toolchain == XTENSA_TOOLCHAIN);
            // Empty exactly when the board declares no partition table, so the
            // workflow can branch on emptiness alone.
            assert_eq!(entry.partitions.is_empty(), board.partitions.is_none());
        }
    }

    #[test]
    fn json_is_a_flat_matrix_include() {
        let json = serde_json::to_value(Targets::collect()).unwrap();

        // GitHub Actions matrix entries have to be scalars, so every value in a
        // board entry must be a string or a bool, never nested.
        for board in json["boards"].as_array().unwrap() {
            for (key, value) in board.as_object().unwrap() {
                assert!(
                    value.is_string() || value.is_boolean(),
                    "{key} is neither a string nor a bool"
                );
            }
        }
    }
}
