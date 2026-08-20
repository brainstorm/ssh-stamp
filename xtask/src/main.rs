// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `ssh_stamp` xtask runner.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod board;
mod cmd;
mod device;
mod elf;
mod host;
mod record;
mod results;
mod stack_probe;
mod stats;

#[derive(Parser)]
#[command(
    name = "xtask",
    about = "ssh-stamp xtask runner",
    version,
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Benchmark ssh-stamp from boot startup to sessions.
    Bench(cmd::bench::Args),
    /// Convert benchmark results.json into Bencher Metric Format.
    Bmf(cmd::bmf::Args),
    /// Determine the size of a firmware build.
    Size(cmd::size::Args),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Bench(mut args) => cmd::bench::run(&mut args),
        Command::Bmf(args) => cmd::bmf::run(&args),
        Command::Size(args) => cmd::size::run(&args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }

    #[allow(clippy::result_large_err)]
    fn size_cmd(argv: &[&str]) -> Result<cmd::size::Args, clap::Error> {
        Cli::try_parse_from(["xtask", "size"].into_iter().chain(argv.iter().copied())).map(|cli| {
            match cli.command {
                Command::Size(parsed) => parsed,
                _ => unreachable!(),
            }
        })
    }

    #[allow(clippy::result_large_err)]
    fn bench_cmd(argv: &[&str]) -> Result<cmd::bench::Args, clap::Error> {
        Cli::try_parse_from(["xtask", "bench"].into_iter().chain(argv.iter().copied())).map(
            |cli| match cli.command {
                Command::Bench(parsed) => parsed,
                _ => unreachable!(),
            },
        )
    }

    #[test]
    fn size_board_names() {
        let args = size_cmd(&["--board", "esp32c5-devkitc"]).unwrap();
        assert_eq!(args.boards[0].name, "esp32c5-devkitc");
        assert_eq!(args.boards[0].soc, "esp32c5");

        assert!(size_cmd(&["--board", "esp32c9"]).is_err());
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
        assert!(bench_cmd(&["--board", "esp32c6-devkitc", "--kex", "c", "--sessions", "0"]).is_err());
        assert!(bench_cmd(&["--board", "esp32c6-devkitc", "--kex", "c", "--rtt-iters", "0"]).is_err());
    }
}
