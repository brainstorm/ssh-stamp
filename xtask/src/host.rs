// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The host side of the device communication.

use crate::util::retry_until;
use anyhow::{Context, Result, bail};
use quick_xml::escape::escape;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use xshell::{Shell, cmd};

/// The device access point.
pub struct AccessPoint {
    /// The access point SSID.
    pub ssid: String,
    /// The access point PSK.
    pub psk: String,
}

impl AccessPoint {
    pub const REACHABLE_TIMEOUT: Duration = Duration::from_mins(1);
    pub const REJOIN_INTERVAL: Duration = Duration::from_secs(8);
    pub const TCP_CONNECT_INTERVAL: Duration = Duration::from_millis(500);

    /// Joins the access point with the ssid and psk. The `interface` can optionally be
    /// specified on multi interface hosts to avoid automatically detecting. It is
    /// required when falling back to `iwctl` on Linux.
    pub fn join(&self, interface: Option<&str>) -> Result<()> {
        let shell = Shell::new()?;
        let (ssid, psk) = (&self.ssid, &self.psk);
        if cfg!(windows) {
            self.add_wlan_profile(&shell)?;
            let name = format!("name={ssid}");
            let interface = interface.map(|interface| format!("interface={interface}"));

            return cmd!(shell, "netsh wlan connect {name} {interface...}")
                .run()
                .context("netsh wlan connect");
        }

        if cfg!(target_os = "macos") {
            let device = interface.unwrap_or("en0");

            return cmd!(
                shell,
                "networksetup -setairportnetwork {device} {ssid} {psk}"
            )
            .secret()
            .quiet()
            .run()
            .context("networksetup -setairportnetwork");
        }

        let ifname = interface
            .map(|interface| vec!["ifname".to_string(), interface.to_string()])
            .unwrap_or_default();

        let nmcli = cmd!(
            shell,
            "nmcli device wifi connect {ssid} password {psk} {ifname...}"
        )
        .secret()
        .quiet()
        .run();

        if let Err(err) = nmcli {
            let Some(station) = interface else {
                bail!(
                    "nmcli failed with `{err}`, it's possible to use iwctl but `--interface` must be specified"
                );
            };

            return cmd!(
                shell,
                "iwctl --passphrase {psk} station {station} connect {ssid}"
            )
            .secret()
            .quiet()
            .run()
            .with_context(|| format!("iwctl fallback after nmcli failed with `{err}`"));
        }

        Ok(())
    }

    /// Imports a profile for `ssid` into the Windows WLAN storage.
    fn add_wlan_profile(&self, shell: &Shell) -> Result<()> {
        // Windows annoyingly doesn't have a convenient way to connect to a Wi-Fi AP
        // programatically. An XML profile must be created which contains the SSID and PSK.
        let mut file = tempfile::Builder::new()
            .suffix(".xml")
            .tempfile()
            .context("creating WLAN profile")?;

        file.write_all(Self::wlan_profile_xml(&self.ssid, &self.psk).as_bytes())
            .context("writing WLAN profile")?;
        let path = file.into_temp_path();

        let filename = format!("filename={}", path.display());
        cmd!(shell, "netsh wlan add profile {filename} user=current")
            .run()
            .context("importing WLAN profile")
    }

    /// The Windows WLAN profile document for a network. Windows requires creating an XML document
    /// for each new network to connect to it.
    fn wlan_profile_xml(ssid: &str, psk: &str) -> String {
        // Escape these because otherwise they may be rejected by netsh.
        let ssid = escape(ssid);
        let psk = escape(psk);
        format!(
            r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
    <name>{ssid}</name>
    <SSIDConfig>
        <SSID>
            <name>{ssid}</name>
        </SSID>
    </SSIDConfig>
    <connectionType>ESS</connectionType>
    <connectionMode>manual</connectionMode>
    <MSM>
        <security>
            <authEncryption>
                <authentication>WPA2PSK</authentication>
                <encryption>AES</encryption>
            </authEncryption>
            <sharedKey>
                <keyType>passPhrase</keyType>
                <protected>false</protected>
                <keyMaterial>{psk}</keyMaterial>
            </sharedKey>
        </security>
    </MSM>
</WLANProfile>"#
        )
    }

    /// Waits until the host can reach the device, i.e. when it `host:22` accepts the TCP connection.
    pub fn wait_for_reachable(&self, host: &str, interface: Option<&str>) -> bool {
        retry_until(Self::REACHABLE_TIMEOUT, Self::REJOIN_INTERVAL, || {
            let tcp_port_open = (host, 22).to_socket_addrs().is_ok_and(|mut addrs| {
                addrs.any(|addr| {
                    TcpStream::connect_timeout(&addr, Self::TCP_CONNECT_INTERVAL).is_ok()
                })
            });
            if tcp_port_open {
                return true;
            }

            match self.join(interface) {
                Ok(()) => eprintln!("=== asked this host to join {} ===", self.ssid),
                Err(err) => {
                    eprintln!("warning: could not join {}: {err:#}", self.ssid);
                }
            }
            false
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_expected_value() {
        let xml = AccessPoint::wlan_profile_xml("ssh-stamp\"", "p&ss");
        assert!(xml.contains("<name>ssh-stamp&quot;</name>"), "{xml}");
        assert!(xml.contains("<keyMaterial>p&amp;ss</keyMaterial>"), "{xml}");
        assert!(!xml.contains("{ssid}") && !xml.contains("{psk}"));
        assert!(xml.contains("<connectionMode>manual</connectionMode>"));
    }
}
