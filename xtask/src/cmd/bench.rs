// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask bench`
//!
//! This builds, flashes and runs SSH sessions to capture `@BENCH` lines. These results
//! are written as JSON.

use crate::board::{self, Board};
use crate::cmd::{self, BENCH_FEATURES};
use crate::device::{self, Serial};
use crate::elf::StackRegion;
use crate::host::AccessPoint;
use crate::record::{self, Record};
use crate::results::{BenchResults, BenchRun, BootCheckpoint, HeapSnapshot, Results, RunOutcome};
use crate::stack_probe::StackProbe;
use crate::stats::{Stats, fmt_bytes, fmt_us, to_f64};
use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use std::env::home_dir;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

/// The variable that overrides the heap size when flashing the firmware.
const HEAP_ENV_VAR: &str = "SSH_STAMP_CONFIG_HEAP_SIZE";

/// The variable for the public key.
const PUBKEY_ENV_VAR: &str = "SSH_STAMP_PUBKEY";

#[derive(ClapArgs)]
pub struct Args {
    /// Board to build and flash.
    #[arg(long, value_parser = board::name_parser())]
    board: &'static Board,
    /// Device IP address.
    #[arg(long, default_value = "192.168.4.1")]
    host: String,
    /// SSH username.
    #[arg(long, default_value = "root")]
    user: String,
    /// The public key used for enrolment on the first boot. Defaults to
    /// `~/.ssh/id_ed25519.pub`.
    #[arg(long)]
    pubkey: Option<PathBuf>,
    /// Number of SSH sessions to execute, this controls how many samples are collected
    /// from an SSH session.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]
    sessions: u32,
    /// Heap sizes to try in bytes, comma separated. The bench will rebuild
    /// and attempt to run each value on the board. Uses the default value if unspecified.
    #[arg(long, value_delimiter = ',')]
    pub(crate) heap: Vec<u64>,
    /// The serial port.
    #[arg(long)]
    port: Option<String>,
    /// The wireless interface that joins the device's access point, e.g.
    /// `wlan0`, `en1` or `Wi-Fi`. This is only needed if the host has more
    /// than one interface.
    #[arg(long)]
    interface: Option<String>,
    /// The kex algorithm every session uses, e.g. `curve25519-sha256`
    /// or `mlkem768x25519-sha256`.
    #[arg(long)]
    kex: String,
    /// The bridge round trips to measure per session, after a few unmeasured
    /// warm-ups.
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..))]
    rtt_iters: u32,
    /// Write results JSON here.
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    /// Echo device serial output to stdout while running.
    #[arg(long)]
    verbose: bool,
}

/// The firmware features to build for `board`.
fn features(board: &Board) -> String {
    board.features(BENCH_FEATURES)
}

impl Args {
    /// Extra ssh settings every session has.
    fn ssh_opts(&self) -> Vec<String> {
        vec![format!("KexAlgorithms={}", self.kex)]
    }

    /// The heap overrides to run.
    fn heaps(&mut self) -> Vec<Option<u64>> {
        if self.heap.is_empty() {
            return vec![None];
        }

        self.heap.sort_unstable();
        self.heap.dedup();

        self.heap.iter().copied().map(Some).collect()
    }
}

pub fn run(args: &mut Args) -> Result<()> {
    let port = Serial::resolve_port(args.port.as_deref())?;
    let features = features(args.board);

    let mut runs = Vec::new();
    for heap_size in args.heaps() {
        runs.push(measure(args, &features, &port, heap_size)?);
    }

    if runs.iter().all(|r| r.bench().is_none()) {
        // This likely indicates that everything failed, not just a heap flash.
        bail!("no run was completed");
    }

    if !args.heap.is_empty() {
        println!();
        for run in &runs {
            let value = run.heap_size().expect("expected heap size");
            match run {
                RunOutcome::Ok { .. } => println!("heap_size = {value}: OK"),
                RunOutcome::Failed { ready, .. } => {
                    println!("heap_size = {value}: FAILED (ready={ready})");
                }
            }
        }

        if let Some(v) = smallest_working(&runs) {
            println!("smallest working heap_size = {v}");
        }
    }

    if let Some(out) = &args.output {
        Results::Bench(BenchResults { features, runs }).write(out)?;
    }

    Ok(())
}

/// The smallest value that was able to complete.
fn smallest_working(runs: &[RunOutcome]) -> Option<u64> {
    runs.iter()
        .filter(|r| r.bench().is_some())
        .filter_map(RunOutcome::heap_size)
        .min()
}

/// Runs the benches for one heap value and returns the measurements.
fn measure(args: &Args, features: &str, port: &str, heap: Option<u64>) -> Result<RunOutcome> {
    let board = args.board;
    let profile = board::PROFILE;

    match heap {
        Some(v) => eprintln!(
            "\n=== {} heap_size = {v}, features: {features} ===",
            board.name
        ),
        None => eprintln!("\n=== {} features: {features} ===", board.name),
    }

    let env: Vec<(String, String)> = heap
        .map(|v| (HEAP_ENV_VAR.to_string(), v.to_string()))
        .into_iter()
        .collect();

    board.build(&xshell::Shell::new()?, profile, features, &env)?;
    device::flash(board, profile, port)?;

    // The flash ends has a reset, and the open must retry.
    let serial = Serial::open(port, args.verbose)?;

    // Capture the boot before the probe-rs painting reboots the chip.
    eprintln!("=== waiting for device to be ready ===");
    if !serial.wait_for_ready(device::BOOT_TIMEOUT) {
        serial.report_health();
        eprintln!("=== device did not become ready ===");
        return Ok(RunOutcome::Failed {
            heap_size: heap,
            ready: false,
        });
    }
    eprintln!("=== ready ===");

    // Paint the stack over the debug link.
    let mut probe = match StackProbe::attach(board.soc, StackRegion::new(&board.elf_path(profile))?)
    {
        Ok(mut probe) => {
            probe.paint()?;
            probe.run()?;
            Some(probe)
        }
        Err(e) => {
            eprintln!("=== no debug link, skipping the stack measurement: {e:#} ===");
            None
        }
    };

    let access_point = AccessPoint::parse(&serial.current_capture())?;
    if !access_point.wait_for_reachable(&args.host, args.interface.as_deref()) {
        eprintln!("=== the device reports itself ready but is unreachable ===");
        return Ok(RunOutcome::Failed {
            heap_size: heap,
            ready: true,
        });
    }
    sleep(Duration::from_secs(1));

    if !enrol(args)? {
        return Ok(RunOutcome::Failed {
            heap_size: heap,
            ready: true,
        });
    }
    let (established, rtt_us) = run_sessions(args)?;

    sleep(Duration::from_secs(2));
    serial.report_health();
    let lines = serial.current_capture();

    if established == 0 || rtt_us.is_empty() {
        return Ok(RunOutcome::Failed {
            heap_size: heap,
            ready: true,
        });
    }

    let mut run = reconstruct(args.kex.clone(), rtt_us, &lines);
    if let Some(probe) = probe.as_mut() {
        run.stack.push(probe.max_usage()?);
    }
    print_summary(&run);

    Ok(RunOutcome::Ok {
        heap_size: heap,
        run,
    })
}

/// Runs the SSH sessions and collects the round-trip samples.
fn run_sessions(args: &Args) -> Result<(u32, Vec<u64>)> {
    let opts = args.ssh_opts();
    let mut established = 0;
    let mut failures = 0;
    let mut timeouts = 0;
    let mut rtt_us: Vec<u64> = Vec::new();
    eprintln!(
        "=== running {} SSH sessions to {}@{}, {} round trips each ===",
        args.sessions, args.user, args.host, args.rtt_iters
    );
    for i in 1..=args.sessions {
        let session =
            device::SessionReport::ssh_session(&args.host, &args.user, &opts, &[], args.rtt_iters)?;
        eprintln!(
            "  session {i:2}: {}, {} of {} markers returned",
            if session.established { "OK" } else { "FAILED" },
            session.rtt_us.len(),
            args.rtt_iters
        );
        if session.established {
            established += 1;
        } else {
            failures += 1;
        }
        timeouts += session.timeouts;

        rtt_us.extend(&session.rtt_us);

        if !session.established && i == 1 {
            eprintln!("=== first session failed ===");
            break;
        }
        if session.established && rtt_us.is_empty() {
            eprintln!(
                "=== nothing came back in the first session, {} timed out ===",
                session.timeouts
            );
            break;
        }

        sleep(Duration::from_secs(1));
    }
    eprintln!("=== sessions done, {failures} failures, {timeouts} timeouts ===");

    Ok((established, rtt_us))
}

/// Enrols the public key into the device on first boot. Returns false if the
/// enrolment session could not be established.
fn enrol(args: &Args) -> Result<bool> {
    let Some((path, pubkey)) = read_pubkey(args.pubkey.as_deref())? else {
        return Ok(true);
    };

    eprintln!("=== adding {} for public key enrolment ===", path.display());
    let mut opts = args.ssh_opts();
    opts.push(format!("SendEnv={PUBKEY_ENV_VAR}"));
    let envs = [(PUBKEY_ENV_VAR.to_string(), pubkey)];

    let session = device::SessionReport::ssh_session(&args.host, &args.user, &opts, &envs, 0)?;
    if !session.established {
        eprintln!("=== the public key failed to enrol ===");
        return Ok(false);
    }

    Ok(true)
}

/// Reads the key to enrol.
fn read_pubkey(path: Option<&Path>) -> Result<Option<(PathBuf, String)>> {
    let path = if let Some(path) = path {
        path.to_path_buf()
    } else {
        let Some(home) = home_dir() else {
            eprintln!("=== no home directory, skipping enrolment ===");
            return Ok(None);
        };
        let default = home.join(".ssh").join("id_ed25519.pub");
        if !default.exists() {
            eprintln!(
                "=== {} not found, skipping enrolment ===",
                default.display()
            );
            return Ok(None);
        }
        default
    };

    let key = fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?
        .trim()
        .to_string();

    if !key.starts_with("ssh-ed25519") {
        bail!("{} is not an Ed25519 key", path.display());
    }

    Ok(Some((path, key)))
}

/// Get the records from captured serial lines and reconstruct them.
fn reconstruct(kex_algorithm: String, rtt_us: Vec<u64>, lines: &[String]) -> BenchRun {
    let records = record::parse_all(lines.iter().map(String::as_str));

    let mut boot: Vec<BootCheckpoint> = Vec::new();
    let mut heap: Vec<HeapSnapshot> = Vec::new();
    let mut kex_us: Vec<u64> = Vec::new();

    for r in &records {
        match r {
            Record::Checkpoint { name, t_us } => {
                // Only the startup checkpoints are recorded.
                if cmd::is_boot_checkpoint(name) && !boot.iter().any(|b| &b.label == name) {
                    boot.push(BootCheckpoint {
                        label: name.clone(),
                        t_abs_us: *t_us,
                    });
                }
            }
            Record::Kex { elapsed_us } => kex_us.push(*elapsed_us),
            Record::Heap {
                label,
                used_bytes,
                total_bytes,
                max_bytes,
            } => {
                if !heap.iter().any(|h| &h.label == label) {
                    heap.push(HeapSnapshot {
                        label: label.clone(),
                        used_bytes: *used_bytes,
                        total_bytes: *total_bytes,
                        max_bytes: *max_bytes,
                    });
                }
            }
        }
    }

    // Order startup checkpoints so output is consistent.
    boot.sort_by_key(|b| {
        cmd::BOOT_CHECKPOINTS
            .iter()
            .position(|c| *c == b.label)
            .unwrap_or(usize::MAX)
    });

    BenchRun {
        kex_algorithm,
        boot,
        heap,
        stack: Vec::new(),
        kex_us,
        rtt_us,
    }
}

/// Print the summary stats.
fn print_summary(r: &BenchRun) {
    match r.boot_t(cmd::TCP_LISTENING) {
        Some(t) => println!("boot to SSH ready: {}", fmt_us(to_f64(t))),
        None => println!("boot: no startup checkpoints"),
    }
    match Stats::from_micros(&r.kex_us) {
        Some(s) => println!(
            "kex: accept to first auth for {}: n={} median={}",
            r.kex_algorithm,
            s.n,
            fmt_us(s.median)
        ),
        None if r.rtt_us.is_empty() => {
            println!("kex: no `@BENCH kex=` line");
        }
        None => {}
    }
    if let Some(s) = Stats::from_micros(&r.rtt_us) {
        println!(
            "round trip on loopback: n={} median={}",
            s.n,
            fmt_us(s.median),
        );
    }
    if let Some(s) = r.stack_max() {
        println!(
            "max stack usage: {} of {} reserved",
            fmt_bytes(s.max_bytes),
            fmt_bytes(s.reserved_bytes)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(heap: Vec<u64>) -> Args {
        Args {
            board: board::find("esp32c6-devkitc").unwrap(),
            host: String::new(),
            user: String::new(),
            pubkey: None,
            sessions: 1,
            heap,
            port: None,
            interface: None,
            kex: String::new(),
            rtt_iters: 1,
            output: None,
            verbose: false,
        }
    }

    #[test]
    fn heaps_are_sorted_and_deduped() {
        assert_eq!(args(vec![]).heaps(), vec![None]);
        assert_eq!(
            args(vec![65_536, 49_152, 49_152]).heaps(),
            vec![Some(49_152), Some(65_536)]
        );
    }

    #[test]
    fn smallest_working_function() {
        let runs = vec![
            RunOutcome::Failed {
                heap_size: Some(49_152),
                ready: false,
            },
            RunOutcome::Failed {
                heap_size: Some(57_344),
                ready: true,
            },
            RunOutcome::Ok {
                heap_size: Some(65_536),
                run: reconstruct(cmd::REFERENCE_KEX.into(), vec![], &[]),
            },
            RunOutcome::Ok {
                heap_size: Some(73_728),
                run: reconstruct(cmd::REFERENCE_KEX.into(), vec![], &[]),
            },
        ];
        assert_eq!(smallest_working(&runs), Some(65_536));

        let runs = vec![RunOutcome::Failed {
            heap_size: Some(32_768),
            ready: false,
        }];
        assert_eq!(smallest_working(&runs), None);
    }
}
