// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Platform-agnostic application entry points.
//!
//! Once a platform crate has brought up its peripherals, loaded the
//! [`SSHStampConfig`] from flash, and raised an `embassy_net::Stack` via a
//! [`ssh_stamp_hal::NetworkProviderHal`] implementation, it hands control
//! here. Everything from "accept a TCP connection" downward is the same on
//! every MCU.

use core::result::Result;

use embassy_futures::select::{Either3, select3};
use embassy_net::{
    IpListenEndpoint, Stack,
    tcp::{AcceptError, TcpSocket},
};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel};
use heapless::String;
use log::{debug, error, info, warn};
use ssh_stamp_hal::{BandMode, WifiApConfigStatic};
#[cfg(feature = "can")]
use sunset::ChanHandle;
use sunset::SignKey;
use sunset_async::SunsetMutex;
#[cfg(feature = "mem-probe")]
use {
    core::pin::pin,
    embassy_futures::select::{Either, select},
    embassy_time::Timer,
};

use crate::config::SSHStampConfig;
use crate::handle::{self, SessionType};
use crate::mem_probe::{Checkpoint, checkpoint, mark_kex_start, replay_checkpoints};
use crate::platform::PlatformServices;
use crate::serial::BufferedSerial;
use crate::serve;
use crate::settings::{
    SSH_STAMP_IDENT, TCP_RX_BUF, TCP_TX_BUF, UART_BUFFER_SIZE, WIFI_PASSWORD_CHARS,
};

/// Ensures a `WiFi` password exists, persists a freshly-generated one if not,
/// prints the SSH hostkey fingerprint, and returns a ready-to-use
/// [`WifiApConfigStatic`] for a [`ssh_stamp_hal::WifiHal`] implementation.
///
/// The returned config resolves the `[0xFF; 6]` random-MAC sentinel to a
/// freshly-generated locally-administered MAC.
///
/// # Errors
///
/// Returns an error if persisting a freshly-minted `WiFi` password fails or
/// MAC resolution fails.
///
/// # Panics
///
/// Panics if `wifi_pw` is unexpectedly empty after the guard block above
/// ensures it is populated. This is an internal invariant violation.
pub async fn prepare_ap_config<P: PlatformServices>(
    config: &SunsetMutex<SSHStampConfig>,
    platform: &P,
) -> Result<WifiApConfigStatic, sunset::Error> {
    let mut guard = config.lock().await;

    info!("SSH server ident: {SSH_STAMP_IDENT}");

    if guard.wifi_ap_pw.is_empty() {
        let pw = generate_wifi_password()?;
        warn!("wifi_pw missing from config, generated new password");
        guard.wifi_ap_pw = pw;
        platform
            .save_config(&guard)
            .await
            .map_err(|_| sunset::error::BadUsage.build())?;
    }
    log_ap_credentials(&guard);

    let mac = guard
        .resolve_mac()
        .map_err(|_| sunset::error::BadUsage.build())?;
    info!(
        "WIFI MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    print_hostkey_fingerprint(&guard.hostkey);

    // Resolve band mode from the stored u8 (0=2.4G, 1=5G, 2=Auto).
    // 5GHz is only available on the ESP32-C5; other chips silently fall
    // back to 2.4GHz at the radio level.
    let band = BandMode::from(guard.wifi_ap_band);
    // Channel 1 for 2.4GHz, channel 36 for 5GHz/Auto (esp-radio default).
    let channel = if guard.wifi_ap_band == 0 { 1 } else { 36 };

    info!("WIFI AP band: {band:?} (channel {channel})");

    Ok(WifiApConfigStatic {
        ap_ssid: guard.wifi_ap_ssid.clone(),
        ap_password: guard.wifi_ap_pw.clone(),
        sta_ssid: guard.wifi_sta_ssid.clone(),
        sta_password: guard.wifi_sta_pw.clone(),
        channel,
        band,
        mac,
    })
}

/// Logs the access point credentials.
fn log_ap_credentials(config: &SSHStampConfig) {
    info!("WIFI SSID: {}", config.wifi_ap_ssid);
    info!("WIFI PSK: {}", config.wifi_ap_pw);
}

/// Accepts TCP connections for the server loop.
///
/// When running the benchmarking code, the device should wait for the host to connect before
/// replaying checkpoints, otherwise a race condition occurs and frequently the bench fails
/// and is flaky. This does not actually affect any of the checkpoint measurement times.
#[cfg(feature = "mem-probe")]
struct SessionAcceptor {
    awaiting_first_session: bool,
}

#[cfg(feature = "mem-probe")]
impl SessionAcceptor {
    fn new() -> Self {
        Self {
            awaiting_first_session: true,
        }
    }

    /// Accepts a TCP connection on the socket. This will wait until the host
    /// connects in a loop.
    async fn accept(
        &mut self,
        tcp_socket: &mut TcpSocket<'_>,
        endpoint: IpListenEndpoint,
        config: &SunsetMutex<SSHStampConfig>,
    ) -> Result<(), AcceptError> {
        if !self.awaiting_first_session {
            return tcp_socket.accept(endpoint).await;
        }

        let mut accept = pin!(tcp_socket.accept(endpoint));
        let accepted = loop {
            match select(&mut accept, Timer::after_secs(3)).await {
                Either::First(accepted) => break accepted,
                Either::Second(()) => {
                    replay_checkpoints();
                    log_ap_credentials(&*config.lock().await);
                }
            }
        };

        if accepted.is_ok() {
            self.awaiting_first_session = false;
        }
        accepted
    }
}

/// Accepts TCP connections for the server loop.
#[cfg(not(feature = "mem-probe"))]
struct SessionAcceptor;

#[cfg(not(feature = "mem-probe"))]
impl SessionAcceptor {
    fn new() -> Self {
        Self
    }

    /// Accepts a TCP connection on the socket.
    async fn accept(
        &mut self,
        tcp_socket: &mut TcpSocket<'_>,
        endpoint: IpListenEndpoint,
        _config: &SunsetMutex<SSHStampConfig>,
    ) -> Result<(), AcceptError> {
        tcp_socket.accept(endpoint).await
    }
}

/// Runs the SSH server loop forever: accept TCP, run SSH, bridge to UART,
/// then go round again. Does not return under normal operation.
///
/// # Errors
///
/// Returns an error only on unrecoverable TCP socket initialisation failure.
pub async fn run_app<U, P>(
    stack: Stack<'static>,
    uart: &U,
    config: &'static SunsetMutex<SSHStampConfig>,
    platform: &P,
) -> Result<(), sunset::Error>
where
    U: BufferedSerial,
    P: PlatformServices,
{
    let mut rx_buffer = [0u8; TCP_RX_BUF];
    let mut tx_buffer = [0u8; TCP_TX_BUF];

    checkpoint(Checkpoint::TcpListening);
    replay_checkpoints();
    log_ap_credentials(&*config.lock().await);
    let mut acceptor = SessionAcceptor::new();
    loop {
        debug!("HSM: accepting TCP on port 22");
        let mut tcp_socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        let endpoint = IpListenEndpoint {
            addr: None,
            port: 22,
        };

        if let Err(e) = acceptor.accept(&mut tcp_socket, endpoint, config).await {
            error!("TCP accept error: {e:?}");
            continue;
        }
        debug!("HSM: TCP connected on port 22");

        mark_kex_start();
        checkpoint(Checkpoint::TcpAccept);

        let mut inbuf = [0u8; UART_BUFFER_SIZE];
        let mut outbuf = [0u8; UART_BUFFER_SIZE];
        let ssh_server = serve::ssh_wait_for_initialisation(&mut inbuf, &mut outbuf);

        let chan_pipe = Channel::<NoopRawMutex, SessionType, 1>::new();
        #[cfg(feature = "can")]
        let can_queue = Channel::<NoopRawMutex, ChanHandle, 1>::new();
        #[cfg(feature = "can")]
        let connection =
            serve::connection_loop(&ssh_server, &chan_pipe, config, platform, &can_queue);
        #[cfg(not(feature = "can"))]
        let connection = serve::connection_loop(&ssh_server, &chan_pipe, config, platform);
        #[cfg(feature = "can")]
        let bridge = handle::ssh_client(uart, &ssh_server, &chan_pipe, platform, &can_queue);
        #[cfg(not(feature = "can"))]
        let bridge = handle::ssh_client(uart, &ssh_server, &chan_pipe, platform);

        let (mut rsock, mut wsock) = tcp_socket.split();
        let server = ssh_server.run(&mut rsock, &mut wsock);

        match select3(server, connection, bridge).await {
            Either3::First(r) | Either3::Second(r) | Either3::Third(r) => {
                if let Err(e) = r {
                    warn!("Session ended: {e}");
                }
            }
        }
    }
}

fn generate_wifi_password() -> Result<String<63>, sunset::Error> {
    let mut rnd = [0u8; 24];
    getrandom::fill(&mut rnd).map_err(|_| sunset::Error::msg("RNG failed"))?;
    let mut pw = String::<63>::new();
    for &byte in &rnd {
        let _ = pw.push(WIFI_PASSWORD_CHARS[(byte as usize) % 62] as char);
    }
    Ok(pw)
}

fn print_hostkey_fingerprint(hostkey: &SignKey) {
    match hostkey {
        SignKey::Ed25519(_) => {
            let pubkey = hostkey.pubkey();
            match pubkey.fingerprint() {
                Ok(fp) => info!("SSH hostkey fingerprint: {fp}"),
                Err(e) => warn!("Failed to compute fingerprint: {e:?}"),
            }
        }
        SignKey::AgentEd25519(_) => {
            warn!("Unsupported key type for fingerprint");
        }
    }
}
