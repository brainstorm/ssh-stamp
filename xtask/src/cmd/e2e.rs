// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask e2e`
//!
//! Runs e2e scenarios from YAML steps.

use crate::board::{self, Board};
use crate::device::Serial;
use crate::host::AccessPoint;
use crate::util::workspace_root;
use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use serde::Deserialize;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use strip_ansi_escapes::strip_str;
use xshell::{Shell, cmd};

const SERIAL_PARSE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Deserialize)]
struct Scenario {
    actions: Vec<Action>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    RestartTargetDevice,
    ParseSshStampNetworkFromSerial,
    ConnectToSsidUsingPsk,
    WaitForReachable,
}

#[derive(Debug, Clone)]
struct NetworkInfo {
    ssid: String,
    psk: String,
    ip: Ipv4Addr,
}

#[derive(Default)]
struct ContextState {
    network: Option<NetworkInfo>,
}

#[derive(ClapArgs)]
pub struct Args {
    /// Board to run end-to-end integration testing on.
    #[arg(long, value_parser = board::name_parser())]
    board: &'static Board,
    /// The serial device used to program/reset and read logs (prg_serial).
    #[arg(long)]
    prg_serial: Option<String>,
    /// The wireless interface that joins the device's AP, needed on multi-NIC hosts.
    #[arg(long)]
    interface: Option<String>,
    /// YAML scenario file to execute.
    #[arg(long)]
    scenario: Option<PathBuf>,
    /// Echo serial output while running.
    #[arg(long)]
    verbose: bool,
}

pub fn run(args: &Args) -> Result<()> {
    let scenario_path = args.scenario.clone().unwrap_or_else(|| {
        workspace_root()
            .join("xtask")
            .join("e2e")
            .join("wifi_ap_mode.yml")
    });
    let scenario = load_scenario(&scenario_path)?;
    let port = Serial::resolve_port(args.prg_serial.as_deref())?;

    let serial = Serial::open(&port, args.verbose)?;
    let mut context = ContextState::default();
    for action in scenario.actions {
        match action {
            Action::RestartTargetDevice => restart_target_device(args, &port, &mut context)?,
            Action::ParseSshStampNetworkFromSerial => {
                parse_ssh_stamp_network_from_serial(&serial, &mut context)?
            }
            Action::ConnectToSsidUsingPsk => connect_to_ssid_using_psk(args, &mut context)?,
            Action::WaitForReachable => wait_for_reachable(args, &mut context)?,
        }
    }
    serial.report_health();
    Ok(())
}

fn restart_target_device(args: &Args, port: &str, _context: &mut ContextState) -> Result<()> {
    let shell = Shell::new()?;
    let soc = args.board.soc;
    cmd!(shell, "espflash reset --port {port} --chip {soc}")
        .run()
        .context("espflash reset failed")
}

fn parse_ssh_stamp_network_from_serial(serial: &Serial, context: &mut ContextState) -> Result<()> {
    let info = parse_network_from_serial(serial, SERIAL_PARSE_TIMEOUT)?;
    eprintln!("parsed SSID={} and IP={}", info.ssid, info.ip);
    context.network = Some(info);
    Ok(())
}

fn connect_to_ssid_using_psk(args: &Args, context: &mut ContextState) -> Result<()> {
    let network = context
        .network
        .as_ref()
        .context("network info is not parsed yet")?;
    AccessPoint {
        ssid: network.ssid.clone(),
        psk: network.psk.clone(),
    }
    .join(args.interface.as_deref())
}

fn wait_for_reachable(args: &Args, context: &mut ContextState) -> Result<()> {
    let network = context
        .network
        .as_ref()
        .context("network info is not parsed yet")?;
    let access_point = AccessPoint {
        ssid: network.ssid.clone(),
        psk: network.psk.clone(),
    };
    if access_point.wait_for_reachable(&network.ip.to_string(), args.interface.as_deref()) {
        Ok(())
    } else {
        bail!("{} did not become reachable", network.ip)
    }
}

fn load_scenario(path: &PathBuf) -> Result<Scenario> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_yaml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

fn parse_network_from_serial(serial: &Serial, timeout: Duration) -> Result<NetworkInfo> {
    let deadline = Instant::now() + timeout;
    loop {
        let lines = serial.current_capture();
        if let Some(parsed) = parse_network_lines(&lines) {
            return Ok(parsed);
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for SSID, PSK and IP in serial output");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn parse_network_lines(lines: &[String]) -> Option<NetworkInfo> {
    let mut ssid: Option<String> = None;
    let mut psk: Option<String> = None;
    let mut ip: Option<Ipv4Addr> = None;

    for raw in lines {
        let line = strip_str(raw);
        if let Some(v) = line.split_once("WIFI SSID:").map(|(_, v)| v.trim()) {
            if !v.is_empty() {
                ssid = Some(v.to_string());
            }
        }
        if let Some(v) = line.split_once("WIFI PSK:").map(|(_, v)| v.trim()) {
            if !v.is_empty() {
                psk = Some(v.to_string());
            }
        }
        if let Some(v) = line.split_once(" with IP ").map(|(_, v)| v.trim()) {
            let token = v.split_whitespace().next().unwrap_or_default();
            let token = token.split('/').next().unwrap_or_default();
            if let Ok(addr) = token.parse::<Ipv4Addr>() {
                ip = Some(addr);
            }
        }
    }

    Some(NetworkInfo {
        ssid: ssid?,
        psk: psk?,
        ip: ip?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_serial_wifi_lines() {
        let lines = vec![
            "\u{1b}[32mINFO - WIFI SSID: test-ssid\u{1b}[0m".to_string(),
            "INFO - WIFI PSK: test-password".to_string(),
            "INFO - Connect to the AP `test-ssid` with IP 192.168.4.1/24".to_string(),
        ];
        let parsed = parse_network_lines(&lines).unwrap();
        assert_eq!(parsed.ssid, "test-ssid");
        assert_eq!(parsed.psk, "test-password");
        assert_eq!(parsed.ip, "192.168.4.1".parse::<Ipv4Addr>().unwrap());
    }

    #[test]
    fn parses_wifi_ap_mode_scenario() {
        let scenario = serde_yaml::from_str::<Scenario>(
            r#"
actions:
  - restart_target_device
  - parse_ssh_stamp_network_from_serial
  - connect_to_ssid_using_psk
  - wait_for_reachable
"#,
        )
        .unwrap();
        assert_eq!(scenario.actions.len(), 4);
    }
}
