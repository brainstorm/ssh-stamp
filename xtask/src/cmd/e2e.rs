// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask e2e`
//!
//! Runs a basic end-to-end hardware integration flow: build, flash, join AP and
//! verify SSH sessions.

use crate::board::{self, Board};
use crate::cmd::REFERENCE_KEX;
use crate::device::{self, Mac, Serial, SessionReport};
use crate::provision::Provision;
use anyhow::{Result, bail};
use clap::Args as ClapArgs;
use std::thread::sleep;
use std::time::Duration;
use xshell::Shell;

#[derive(ClapArgs)]
pub struct Args {
    /// Board to run end-to-end integration testing on.
    #[arg(long, value_parser = board::name_parser())]
    board: &'static Board,
    /// Device IP address to reach over Wi-Fi.
    #[arg(long, default_value = "192.168.4.1")]
    host: String,
    /// SSH username.
    #[arg(long, default_value = "root")]
    user: String,
    /// The serial port for espflash commands.
    #[arg(long)]
    port: Option<String>,
    /// The wireless interface that joins the device's AP, needed on multi-NIC hosts.
    #[arg(long)]
    interface: Option<String>,
    /// Number of SSH sessions to validate.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    sessions: u32,
    /// Round trips per SSH session.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]
    rtt_iters: u32,
    /// Echo serial output while running.
    #[arg(long)]
    verbose: bool,
}

pub fn run(args: &Args) -> Result<()> {
    let port = Serial::resolve_port(args.port.as_deref())?;
    let features = args.board.features(&[]);
    args.board
        .build(&Shell::new()?, board::PROFILE, &features, &[])?;

    let mac = Mac::read(args.board, &port)?;
    let provision = Provision::generate(&args.host, mac.into_inner(), args.board.uart_pins()?)?;
    device::flash(args.board, board::PROFILE, &port, provision.image())?;

    let serial = Serial::open(&port, args.verbose)?;
    let access_point = provision.access_point();
    if !access_point.wait_for_reachable(&args.host, args.interface.as_deref()) {
        serial.report_health();
        bail!("device is unreachable on {}", args.host);
    }

    let auth = provision.ssh_auth();
    let opts = vec![format!("KexAlgorithms={REFERENCE_KEX}")];
    for i in 1..=args.sessions {
        let report =
            SessionReport::ssh_session(&args.host, &args.user, &auth, &opts, &[], args.rtt_iters)?;
        if !report.established {
            bail!("SSH session {i} failed");
        }
        if report.rtt_us.is_empty() {
            bail!("SSH session {i} returned no round-trip samples");
        }
        eprintln!(
            "session {i}: {} samples, {} timeouts",
            report.rtt_us.len(),
            report.timeouts
        );
        sleep(Duration::from_secs(1));
    }
    serial.report_health();

    Ok(())
}
