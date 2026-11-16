// SPDX-FileCopyrightText: 2026 Julio Beltran Ortega <jubeormk1@gmail.com>
// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask test e2e`
//!
//! Runs end-to-end scenarios against an already flashed board. A scenario is
//! a plain function calling its steps in order, and the device state it needs
//! comes from the `boot` and `net_up` JSON objects on the serial console.

use crate::board::{self, Board};
use crate::device::{self, Serial};
use crate::host::AccessPoint;
use crate::util::{retry, shell};
use anyhow::{Context, Result, bail, ensure};
use clap::{Args as ClapArgs, ValueEnum};
use serde::Deserialize;
use ssh_stamp::json::SCHEMA;
use std::net::Ipv4Addr;
use std::time::Duration;
use xshell::cmd;

/// How often the serial capture is checked for new events.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(ClapArgs)]
pub struct Args {
    /// Board to run the scenario on.
    #[arg(long, value_parser = board::name_parser())]
    board: &'static Board,
    /// The serial port used to reset the board and read its console.
    #[arg(long)]
    port: Option<String>,
    /// The wireless interface that joins the device's access point, needed on multi-NIC hosts.
    #[arg(long)]
    interface: Option<String>,
    /// The scenario to run.
    #[arg(long, value_enum, default_value_t = Scenario::WifiAp)]
    scenario: Scenario,
    /// Echo serial output while running.
    #[arg(long)]
    verbose: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Scenario {
    /// Reset the board, join the access point it hosts and reach its SSH port.
    WifiAp,
}

pub fn run(args: &Args) -> Result<()> {
    let port = Serial::resolve_port(args.port.as_deref())?;
    let serial = Serial::open(&port, args.verbose)?;

    let result = match args.scenario {
        Scenario::WifiAp => wifi_ap(args, &port, &serial),
    };

    serial.report_health();
    result
}

fn wifi_ap(args: &Args, port: &str, serial: &Serial) -> Result<()> {
    let (boot, net_up) = reset_and_wait_for_network(args.board, port, serial)?;
    ensure!(
        net_up.role == "ap",
        "expected the device to host an access point, but its role is `{}`",
        net_up.role
    );
    eprintln!(
        "=== device hosts {} at {} ===",
        boot.wifi_ap.ssid, net_up.ip
    );

    let access_point = AccessPoint {
        ssid: boot.wifi_ap.ssid,
        psk: boot.wifi_ap.psk,
    };
    ensure!(
        access_point.wait_for_reachable(&net_up.ip.to_string(), args.interface.as_deref()),
        "{} did not become reachable",
        net_up.ip
    );

    eprintln!("=== e2e {:?} passed ===", args.scenario);
    Ok(())
}

/// Resets the board and waits for the `boot` and `net_up` events of the boot
/// that follows, ignoring anything captured before the reset.
fn reset_and_wait_for_network(board: &Board, port: &str, serial: &Serial) -> Result<(Boot, NetUp)> {
    let skip = serial.current_capture().len();
    let shell = shell()?;
    let soc = board.soc;
    cmd!(shell, "espflash reset --port {port} --chip {soc}")
        .run()
        .context("espflash reset failed")?;

    retry(device::BOOT_TIMEOUT, POLL_INTERVAL, || {
        let lines = serial.current_capture();
        let events = events(lines.get(skip..).unwrap_or_default())?;
        let boot = events.iter().find_map(|event| match event {
            Event::Boot(boot) => Some(boot.clone()),
            _ => None,
        });
        let net_up = events.iter().find_map(|event| match event {
            Event::NetUp(net_up) => Some(net_up.clone()),
            _ => None,
        });
        match (boot, net_up) {
            (Some(boot), Some(net_up)) => Ok((boot, net_up)),
            (None, _) => bail!("timed out waiting for the `boot` event"),
            (_, None) => bail!("timed out waiting for the `net_up` event"),
        }
    })
}

/// The version every JSON object the firmware prints starts with, see `ssh_stamp::json`.
#[derive(Debug, Deserialize)]
struct Schema {
    schema: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event {
    Boot(Boot),
    NetUp(NetUp),
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
struct Boot {
    wifi_ap: WifiAp,
}

#[derive(Debug, Clone, Deserialize)]
struct WifiAp {
    ssid: String,
    psk: String,
}

#[derive(Debug, Clone, Deserialize)]
struct NetUp {
    role: String,
    ip: Ipv4Addr,
}

/// Cuts the firmware's JSON objects out of the console `lines`. A top-level
/// object always opens and closes with `{` and `}` alone on their lines, so
/// everything else is console noise. Objects cut short, e.g. by a reset, are
/// skipped.
fn events(lines: &[String]) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    let mut object: Option<String> = None;

    for line in lines {
        match line.as_str() {
            "{" => object = Some(String::from("{")),
            "}" => {
                let Some(mut text) = object.take() else {
                    continue;
                };
                text.push('}');
                let Ok(Schema { schema }) = serde_json::from_str(&text) else {
                    continue;
                };
                ensure!(
                    schema == SCHEMA,
                    "the firmware prints JSON schema {schema}, but xtask understands {SCHEMA}"
                );
                events.push(serde_json::from_str(&text).context("parsing a console event")?);
            }
            _ => {
                if let Some(text) = object.as_mut() {
                    text.push_str(line);
                }
            }
        }
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn parses_console_events() {
        let console = lines(
            r#"ESP-ROM:esp32c6-20220919
{
  "schema": 1,
  "event": "boot",
  "ident": "SSH-2.0-ssh-stamp",
  "wifi_ap": {
    "ssid": "my \"ssid\"",
    "psk": "test-password",
    "band": "2.4GHz",
    "channel": 1
  },
  "first_login": true
}
INFO - unrelated log line
{
  "schema": 1,
  "event": "net_up",
  "role": "ap",
  "ssid": "my \"ssid\"",
  "ip": "10.0.0.1"
}
{
  "schema": 1,
  "event": "log",
  "msg": "ignored"
}"#,
        );
        let events = events(&console).unwrap();
        let [Event::Boot(boot), Event::NetUp(net_up), Event::Other] = events.as_slice() else {
            panic!("unexpected events: {events:?}");
        };
        assert_eq!(boot.wifi_ap.ssid, r#"my "ssid""#);
        assert_eq!(boot.wifi_ap.psk, "test-password");
        assert_eq!(net_up.role, "ap");
        assert_eq!(net_up.ip, Ipv4Addr::new(10, 0, 0, 1));
    }

    #[test]
    fn skips_truncated_objects() {
        let console = lines(
            "{\n  \"schema\": 1,\n  \"event\": \"bo\n{\n  \"schema\": 1,\n  \"event\": \"net_up\",\n  \"role\": \"ap\",\n  \"ip\": \"192.168.4.1\"\n}",
        );
        assert!(matches!(
            events(&console).unwrap().as_slice(),
            [Event::NetUp(_)]
        ));
    }

    #[test]
    fn rejects_unknown_schema() {
        let console = lines("{\n  \"schema\": 999,\n  \"event\": \"boot\"\n}");
        assert!(events(&console).is_err());
    }
}
