// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `ssh_stamp` xtask runner. `cargo xtask <command>` for project tasks,
//! and `cargo xtask <target> <cargo command> [args...]` to run a cargo
//! command against a board or chip.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod board;
mod cmd;
mod device;
mod elf;
mod host;
mod provision;
mod record;
mod results;
mod stack_probe;
mod stats;
mod util;

#[derive(Parser)]
#[command(
    name = "xtask",
    about = "ssh-stamp xtask runner",
    version,
    after_help = "\
Any board or chip name is also a command, where the rest of the line is a cargo command
line, forwarded with the target's toolchain, triple and feature selection applied.

  cargo xtask esp32c6-devkitc build --release
  cargo xtask esp32c6-devkitc run --release --features sftp-ota
  cargo xtask esp32c3 clippy -- -D warnings
  cargo xtask esp32-s2-saola tree -i esp-hal"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List known boards and chips.
    List(cmd::list::Args),
    /// Benchmark ssh-stamp from boot startup to sessions.
    Bench(cmd::bench::Args),
    /// Convert benchmark results.json into Bencher Metric Format.
    Bmf(cmd::bmf::Args),
    /// Determine the size of a firmware build.
    Size(cmd::size::Args),
    /// Resets the storage on board, while keeping the firmware intact.
    Reset(cmd::reset::Args),
    /// `<target> <cargo command> [args...]` forwarded to cargo.
    #[command(external_subcommand)]
    Cargo(Vec<String>),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::List(args) => cmd::list::run(&args),
        Command::Bench(mut args) => cmd::bench::run(&mut args),
        Command::Bmf(args) => cmd::bmf::run(&args),
        Command::Size(args) => cmd::size::run(&args),
        Command::Reset(args) => cmd::reset::run(&args),
        Command::Cargo(argv) => cmd::cargo::run(&argv),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    fn size_cmd(argv: &[&str]) -> Result<cmd::size::Args, Box<clap::Error>> {
        Cli::try_parse_from(["xtask", "size"].into_iter().chain(argv.iter().copied()))
            .map(|cli| match cli.command {
                Command::Size(parsed) => parsed,
                _ => unreachable!(),
            })
            .map_err(Box::new)
    }

    fn bench_cmd(argv: &[&str]) -> Result<cmd::bench::Args, Box<clap::Error>> {
        Cli::try_parse_from(["xtask", "bench"].into_iter().chain(argv.iter().copied()))
            .map(|cli| match cli.command {
                Command::Bench(parsed) => parsed,
                _ => unreachable!(),
            })
            .map_err(Box::new)
    }

    fn reset_cmd(argv: &[&str]) -> Result<cmd::reset::Args, Box<clap::Error>> {
        Cli::try_parse_from(["xtask", "reset"].into_iter().chain(argv.iter().copied()))
            .map(|cli| match cli.command {
                Command::Reset(parsed) => parsed,
                _ => unreachable!(),
            })
            .map_err(Box::new)
    }

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn commands_and_targets_disjoint() {
        let command = Cli::command();
        let commands = command
            .get_subcommands()
            .map(clap::Command::get_name)
            .collect::<Vec<_>>();

        let targets = board::BOARDS
            .iter()
            .map(|b| b.name)
            .chain(board::CHIPS.iter().map(|c| c.name));
        for target in targets {
            assert!(!commands.contains(&target));
        }
    }

    #[test]
    fn forwards_targets_to_cargo() {
        let cli = Cli::try_parse_from([
            "xtask",
            "esp32c6-devkitc",
            "build",
            "--release",
            "--",
            "--timings",
        ])
        .unwrap();
        let Command::Cargo(argv) = cli.command else {
            panic!();
        };
        assert_eq!(
            argv,
            ["esp32c6-devkitc", "build", "--release", "--", "--timings"]
        );

        let cli = Cli::try_parse_from(["xtask", "esp32-fake-name", "build"]).unwrap();
        assert!(matches!(cli.command, Command::Cargo(_)));
        assert!(Cli::try_parse_from(["xtask"]).is_err());
    }

    #[test]
    fn reset() {
        assert!(reset_cmd(&["--board", "esp32c6-devkitc"]).is_ok());
        assert!(
            reset_cmd(&[
                "--board",
                "esp32c6-devkitc",
                "--mode",
                "probe-rs",
                "--erase-otadata"
            ])
            .is_ok()
        );
        assert!(reset_cmd(&["--board", "esp32-fake-name"]).is_err());
        assert!(reset_cmd(&[]).is_err());
    }

    #[test]
    fn size_board_names() {
        let args = size_cmd(&["--board", "esp32c5-devkitc"]).unwrap();
        assert_eq!(args.boards[0].name, "esp32c5-devkitc");
        assert_eq!(args.boards[0].soc, "esp32c5");

        assert!(size_cmd(&["--board", "esp32-fake-name"]).is_err());
        assert!(size_cmd(&["--board", "esp32c3"]).is_err());

        assert!(size_cmd(&["--all"]).is_ok());
        assert!(size_cmd(&["--all", "--board", "esp32c6-devkitc"]).is_err());

        assert!(size_cmd(&[]).is_err());
    }

    #[test]
    fn bench() {
        let args = bench_cmd(&[
            "--board",
            "esp32c6-devkitc",
            "--kex",
            "curve25519-sha256",
            "--heap",
            "49152,57344",
        ])
        .unwrap();
        assert_eq!(args.heap, vec![49_152, 57_344]);

        assert!(bench_cmd(&["--kex", "curve25519-sha256"]).is_err());
        assert!(bench_cmd(&["--board", "esp32c6-devkitc"]).is_err());
        assert!(
            bench_cmd(&[
                "--board",
                "esp32c6-devkitc",
                "--kex",
                "c",
                "--sessions",
                "0"
            ])
            .is_err()
        );
        assert!(
            bench_cmd(&[
                "--board",
                "esp32c6-devkitc",
                "--kex",
                "c",
                "--rtt-iters",
                "0"
            ])
            .is_err()
        );
    }
}
