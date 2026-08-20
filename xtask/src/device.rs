// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The device level implementation for the serial monitor.

use crate::board::Board;
use crate::cmd;
use anyhow::{Context, Result, bail};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use serial2::SerialPort;
use xshell::{Shell, cmd};

/// The SSH `ConnectTimeout` that applies to each session.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The timeout for the device booting.
pub const BOOT_TIMEOUT: Duration = Duration::from_mins(2);

/// What the session gets to set everything up.
pub const SETUP_TIMEOUT: Duration = Duration::from_secs(2);

/// This will flash the firmware onto the board via espflash.
pub fn flash(board: &Board, profile: &str, port: &str) -> Result<()> {
    let shell = Shell::new()?;
    let soc = board.soc;
    let elf = board.elf_path(profile).display().to_string();

    let partitions: Vec<String> = board
        .partitions
        .map(|partition| vec!["--partition-table".to_string(), partition.to_string()])
        .unwrap_or_default();

    eprintln!("=== flashing {elf} to {port} for {soc} ===");

    cmd!(
        shell,
        "espflash flash --port {port} {partitions...} --chip {soc} {elf}"
    )
    .run()
    .context("espflash flash failed")?;

    Ok(())
}

/// The SSH session report.
pub struct SessionReport {
    /// Session was successfully established.
    pub established: bool,
    /// The round trip times.
    pub rtt_us: Vec<u64>,
    /// Any RTT timeouts that weren't successful within [`RTT_MARKER_TIMEOUT`].
    pub timeouts: u32,
}

impl SessionReport {
    const OUT_READER_BUF: usize = 4096;

    /// Open an SSH session and measures the round trip times over it.
    pub fn ssh_session(
        host: &str,
        user: &str,
        extra_opts: &[String],
        markers: u32,
    ) -> Result<Self> {
        let shell = Shell::new()?;
        let mut child = Self::ssh_session_cmd(&shell, host, user, extra_opts, true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("could not run the ssh client")?;

        let stderr = child.stderr.take().expect("stderr was piped");
        let reader = thread::spawn(move || {
            BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
                .collect::<Vec<_>>()
        });

        let mut stdout = child.stdout.take().expect("stdout was piped");
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let out_reader = thread::spawn(move || {
            let mut buf = [0u8; Self::OUT_READER_BUF];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });

        let stdin = child.stdin.take().expect("stdin was piped");
        let trips = RoundTrips::measure_round_trips(stdin, &rx, markers);

        let _ = child.kill();
        let _ = child.wait();
        let trace = reader.join().unwrap_or_default();
        let _ = out_reader.join();

        if !trips.established {
            Self::print_ssh_failure(&trace);
        }

        Ok(SessionReport {
            established: trips.established,
            rtt_us: trips.samples_us,
            timeouts: trips.timeouts,
        })
    }

    /// Print the last ssh error line.
    pub fn print_ssh_failure(trace: &[String]) {
        if let Some(line) = trace.iter().rev().find(|l| !l.starts_with("debug")) {
            eprintln!("           ssh: {}", line.trim());
        }
    }

    /// Creates the `ssh` call that is used between the host and the device.
    fn ssh_session_cmd(
        shell: &Shell,
        host: &str,
        user: &str,
        extra_opts: &[String],
        verbose: bool,
    ) -> Command {
        let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };
        let known_hosts = format!("UserKnownHostsFile={null_device}");
        let connect_timeout = format!("ConnectTimeout={}", CONNECT_TIMEOUT.as_secs());
        let verbose = verbose.then_some("-v");
        let extra_opts: Vec<String> = extra_opts
            .iter()
            .flat_map(|opt| ["-o".to_string(), opt.clone()])
            .collect();
        let destination = format!("{user}@{host}");

        cmd!(
        shell,
        "ssh -T -F none -o BatchMode=yes -o StrictHostKeyChecking=no -o {known_hosts} -o {connect_timeout} {verbose...} {extra_opts...} {destination}"
    )
            .into()
    }
}

/// What one marker represents when it's written from a session.
pub enum Echo {
    /// It came back inside the deadline.
    Returned,
    /// It failed to come back, although the session is still open.
    Lost,
    /// The ssh session has been closed and nothing else can come back.
    Closed,
}

/// The information observed in a single round trip session.
#[derive(Default)]
pub struct RoundTrips {
    /// The session was successfully established.
    established: bool,
    /// The samples from within the session.
    samples_us: Vec<u64>,
    /// Any samples that timed out.
    timeouts: u32,
}

impl RoundTrips {
    const RTT_WARMUP: u32 = 3;
    const RTT_MARKER_TIMEOUT: Duration = CONNECT_TIMEOUT.saturating_add(SETUP_TIMEOUT);

    /// Writes the round trip markers and times each one until it returns.
    pub fn measure_round_trips(
        mut stdin: ChildStdin,
        rx: &mpsc::Receiver<Vec<u8>>,
        markers: u32,
    ) -> Self {
        let mut pending: Vec<u8> = Vec::new();
        let mut samples_us = Vec::new();
        let mut timeouts = 0u32;

        for i in 0..markers + Self::RTT_WARMUP {
            let start = Instant::now();
            let echo = Self::round_trip(
                &mut stdin,
                rx,
                &mut pending,
                &Self::rtt_marker(i),
                start + Self::RTT_MARKER_TIMEOUT,
            );
            let elapsed = start.elapsed();

            // The first marker is the whole handshake, so when it returns unsuccessfully,
            // it means there is no session at all and there is nothing to measure.
            if i == 0 && matches!(echo, Echo::Lost | Echo::Closed) {
                return RoundTrips::default();
            }

            if matches!(echo, Echo::Closed) {
                // The session died, everything else is lost information.
                timeouts += (markers + Self::RTT_WARMUP - i).min(markers);
                break;
            }

            if i < Self::RTT_WARMUP {
                continue;
            }

            match echo {
                Echo::Returned => {
                    samples_us.push(u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX));
                }
                _ => timeouts += 1,
            }
        }

        RoundTrips {
            established: true,
            samples_us,
            timeouts,
        }
    }

    /// Writes `marker` and waits for the echo to come back by `deadline`.
    pub fn round_trip(
        stdin: &mut ChildStdin,
        rx: &mpsc::Receiver<Vec<u8>>,
        pending: &mut Vec<u8>,
        marker: &[u8],
        deadline: Instant,
    ) -> Echo {
        match Self::write_marker(stdin, marker) {
            Ok(()) => Self::await_echo(rx, pending, marker, deadline),
            Err(_) => Echo::Closed,
        }
    }

    /// Writes a marker and pushes it out to the device immediately.
    pub fn write_marker(stdin: &mut ChildStdin, marker: &[u8]) -> io::Result<()> {
        stdin.write_all(marker).and_then(|()| stdin.flush())
    }

    /// Waits for the `marker` to be echoed back up to the `deadline`.
    pub fn await_echo(
        rx: &mpsc::Receiver<Vec<u8>>,
        pending: &mut Vec<u8>,
        marker: &[u8],
        deadline: Instant,
    ) -> Echo {
        loop {
            if let Some(pos) = pending.windows(marker.len()).position(|w| w == marker) {
                pending.drain(..pos + marker.len());
                return Echo::Returned;
            }
            // A deadline that has already passed instantly times out the recv.
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(chunk) => pending.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => return Echo::Lost,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Echo::Closed,
            }
        }
    }

    /// Create the round-trip marker, kept with a constant width to ensure accurate
    /// measurement.
    fn rtt_marker(i: u32) -> Vec<u8> {
        format!("[rtt:{i:010}]").into_bytes()
    }
}

/// What the reader has seen on the port so far.
///
/// One lock over both halves, so a reader of the error also sees the line count
/// that goes with it rather than one from another instant.
#[derive(Default)]
struct Capture {
    /// Every complete line, in the order it arrived.
    lines: Vec<String>,
    /// The read error that closed the port, cleared when it reopens.
    error: Option<String>,
}

/// The background serial reader
pub struct Serial {
    port: String,
    capture: Arc<Mutex<Capture>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Serial {
    const SERIAL_WAIT: Duration = Duration::from_millis(250);
    const CONSOLE_BAUD: u32 = 115_200;
    const READ_BUF: usize = 1024;

    /// Opens the `port` and starts collecting data. The `echo` variable controls
    /// whether each line is also printed.
    pub fn open(port: &str, echo: bool) -> Result<Serial> {
        let serial_port = Self::open_port(port)?;

        let capture = Arc::new(Mutex::new(Capture::default()));
        let stop = Arc::new(AtomicBool::new(false));

        let capture_copy = capture.clone();
        let stop_copy = stop.clone();
        let name = port.to_string();
        let handle = thread::spawn(move || {
            Self::capture(serial_port, &name, echo, &capture_copy, &stop_copy);
        });

        Ok(Serial {
            port: port.to_string(),
            capture,
            stop,
            handle: Some(handle),
        })
    }

    /// Reads the serial port into `out` until `stop` is true.
    fn capture(
        serial_port: SerialPort,
        name: &str,
        echo: bool,
        out: &Mutex<Capture>,
        stop: &AtomicBool,
    ) {
        let mut serial_port = Some(serial_port);
        let mut buf = [0u8; Self::READ_BUF];
        let mut accumulate = String::new();

        while !stop.load(Ordering::Relaxed) {
            let Some(open) = serial_port.as_mut() else {
                thread::sleep(Self::SERIAL_WAIT);

                if let Ok(reopened) = Self::open_port(name) {
                    // The port is back, so the error that closed is no longer valid.
                    Self::lock_capture(out).error = None;
                    serial_port = Some(reopened);
                }

                continue;
            };

            match open.read(&mut buf) {
                Ok(n) => {
                    accumulate.push_str(&String::from_utf8_lossy(&buf[..n]));

                    while let Some(pos) = accumulate.find('\n') {
                        let line = accumulate[..pos].trim_end().to_string();
                        accumulate.drain(..=pos);

                        if echo {
                            println!("{line}");
                        }

                        Self::lock_capture(out).lines.push(line);
                    }
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock) => {}
                Err(e) => {
                    Self::lock_capture(out).error = Some(e.to_string());
                    serial_port = None;
                }
            }
        }
    }


    /// List the available serial ports on the OS.
    pub fn available_ports() -> Result<Vec<PathBuf>> {
        SerialPort::available_ports().context("enumerating serial ports")
    }

    /// Resolves the serial port to use.
    pub fn resolve_port(explicit_port: Option<&str>) -> Result<String> {
        if let Some(p) = explicit_port {
            return Ok(p.to_string());
        }

        let ports = Self::available_ports().unwrap_or_default();
        match ports.as_slice() {
            [one_port] => {
                let name = one_port.display().to_string();
                eprintln!("automatically selected serial port {name}");
                Ok(name)
            }
            [] => bail!("no serial ports found, pass --port explicitly"),
            many_ports => {
                let list = many_ports
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!("multiple serial ports found ({list}), pass --port explicitly")
            }
        }
    }

    /// Opens the port for capture.
    pub fn open_port(port: &str) -> Result<SerialPort> {
        let mut serial_port = SerialPort::open(port, Self::CONSOLE_BAUD)
            .with_context(|| format!("opening serial port {port}"))?;
        serial_port.set_read_timeout(Self::SERIAL_WAIT)
            .context("setting serial read timeout")?;

        Ok(serial_port)
    }

    /// Returns a copy of every line seen so far.
    pub fn current_capture(&self) -> Vec<String> {
        Self::lock_capture(&self.capture).lines.clone()
    }

    /// Waits for the device to be ready and listening.
    pub fn wait_for_ready(&self, timeout: Duration) -> bool {
        let needle = format!("checkpoint={}", cmd::TCP_LISTENING);
        let timeout = Instant::now() + timeout;
        while Instant::now() < timeout {
            // Locked directly rather than through `current_capture`.
            let contains = Self::lock_capture(&self.capture)
                .lines
                .iter()
                .any(|l| l.contains(&needle));

            if contains {
                return true;
            }

            thread::sleep(Self::SERIAL_WAIT);
        }

        false
    }

    /// Reports the current status of the capture.
    pub fn report_health(&self) {
        let (captured, error) = {
            let capture = Self::lock_capture(&self.capture);
            (capture.lines.len(), capture.error.clone())
        };
        let Some(err) = error else {
            if captured == 0 {
                eprintln!("note: nothing was read from {port}", port = self.port);
            }
            return;
        };
        eprintln!("warning: reading {port} failed with {err}", port = self.port);
    }

    /// Lock the capture and return the guard unconditionally.
    fn lock_capture(capture: &Mutex<Capture>) -> MutexGuard<'_, Capture> {
        capture.lock().expect("failed to lock serial capture")
    }
}

impl Drop for Serial {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_split_across_reads() {
        // An echo is valid across reads.
        let (tx, rx) = mpsc::channel();
        let marker = RoundTrips::rtt_marker(4);
        let (head, tail) = marker.split_at(5);
        tx.send(head.to_vec()).unwrap();
        tx.send(tail.to_vec()).unwrap();

        let mut pending = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        assert!(matches!(
            RoundTrips::await_echo(&rx, &mut pending, &marker, deadline),
            Echo::Returned
        ));
        assert!(pending.is_empty());
    }

    #[test]
    fn late_marker() {
        // A marker that timed out and then turned up is left in `pending`.
        let (tx, rx) = mpsc::channel();
        tx.send(RoundTrips::rtt_marker(1)).unwrap();
        drop(tx);

        let mut pending = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        assert!(matches!(
            RoundTrips::await_echo(&rx, &mut pending, &RoundTrips::rtt_marker(2), deadline),
            Echo::Closed
        ));
        assert_eq!(pending, RoundTrips::rtt_marker(1));
    }

    #[test]
    fn silent_session_loses_marker() {
        // Nothing sent and the sender still active
        let (_tx, rx) = mpsc::channel();
        let mut pending = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(50);
        assert!(matches!(
            RoundTrips::await_echo(&rx, &mut pending, &RoundTrips::rtt_marker(0), deadline),
            Echo::Lost
        ));
        assert!(Instant::now() >= deadline);
    }

    #[test]
    fn rtt_markers_fixed_width() {
        let m = RoundTrips::rtt_marker(7);
        assert_eq!(m.len(), 16);
        assert!(!m.contains(&b'\n') && !m.contains(&b'\r'));
        assert_ne!(RoundTrips::rtt_marker(1), RoundTrips::rtt_marker(2));
        let noisy = b"head[rtt:0000000007]tail";
        assert_eq!(noisy.windows(m.len()).position(|w| w == m), Some(4));
        assert_eq!(m.windows(6).position(|w| w == b"[rtt:9"), None);
    }
}
