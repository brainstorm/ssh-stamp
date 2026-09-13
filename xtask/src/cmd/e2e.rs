// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask e2e`
//!
//! Runs the OTA end-to-end hardware integration test script with board-aware defaults.

use crate::board::{self, Board};
use crate::util::{shell, workspace_root};
use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use std::path::PathBuf;
use xshell::cmd;

#[derive(ClapArgs)]
pub struct Args {
    /// Board to run end-to-end integration testing on.
    #[arg(long, value_parser = board::name_parser())]
    board: &'static Board,
    /// Device IP address to reach over Wi-Fi.
    #[arg(long, default_value = "192.168.4.1")]
    host: String,
    /// Expected OTA partition offset seen after update.
    #[arg(long, default_value = "0x1f0000")]
    ota_offset: String,
    /// The serial port for espflash commands.
    #[arg(long)]
    port: Option<String>,
    /// Reachability retries.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..))]
    retries: u32,
    /// Delay between retries in seconds.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u32).range(1..))]
    retry_delay: u32,
    /// Timeout used for OTA upload and monitor checks.
    #[arg(long, default_value = "300s")]
    ota_upload_timeout: String,
    /// Override the script path if needed.
    #[arg(long)]
    script: Option<PathBuf>,
}

pub fn run(args: &Args) -> Result<()> {
    let script = args.script.clone().unwrap_or_else(|| {
        workspace_root()
            .join("ota")
            .join(format!("test-hil-{}-e2e.sh", args.board.soc))
    });
    if !script.exists() {
        bail!("missing e2e script at {}", script.display());
    }

    let sh = shell()?;
    let output_dir = workspace_root()
        .join("target")
        .join("ci")
        .join(args.board.name);

    let mut command = cmd!(sh, "bash {script}");
    command = command
        .env("E2E_BOARD", args.board.name)
        .env("E2E_CHIP", args.board.soc)
        .env(
            "E2E_SSH_STAMP_ELF",
            args.board.elf_path(board::PROFILE).display().to_string(),
        )
        .env("E2E_DEVICE_IP", &args.host)
        .env("E2E_OTA_1_OFFSET", &args.ota_offset)
        .env("E2E_RETRIES", args.retries.to_string())
        .env("E2E_RETRY_DELAY", args.retry_delay.to_string())
        .env("E2E_OTA_UPLOAD_TIMEOUT", &args.ota_upload_timeout)
        .env("E2E_OUTPUT_DIR", output_dir.display().to_string());

    if let Some(port) = &args.port {
        command = command.env("E2E_SERIAL_PORT", port);
    }

    command
        .run()
        .with_context(|| format!("e2e script failed for {}", args.board.name))
}
