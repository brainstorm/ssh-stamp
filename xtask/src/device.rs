// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Subprocess drivers (`cargo` / `espflash` / `ssh`) and a non-blocking serial
//! monitor. xtask never links firmware or probe libraries — it shells out and
//! reads the device's serial output, so it stays a small, cross-platform host
//! tool.

use anyhow::{Context, Result, bail};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use xshell::{Shell, cmd};

/// ESP32-C6 build/flash defaults (the only board the bench flows target today).
pub const TARGET: &str = "riscv32imac-unknown-none-elf";
pub const CHIP: &str = "esp32c6";
pub const PARTITIONS: &str = "ssh-stamp-esp32/partitions.csv";
pub const ELF: &str = "target/riscv32imac-unknown-none-elf/release/ssh-stamp-esp32";

/// The checkpoint line that means "firmware is ready to accept SSH on port 22"
/// — `bench_tcp_listening()` fires immediately before the accept loop.
pub const READY_CHECKPOINT: &str = "checkpoint=bench_tcp_listening";

/// Builds the esp32c6 firmware with the given comma-separated feature string
/// (always includes `mem-probe` so `@BENCH` lines are emitted), optionally with
/// `SSH_STAMP_CONFIG_*` overrides applied for this build only.
///
/// No `cargo clean` is needed before changing an override: the core crate's
/// `build.rs` declares the tunables through `esp-config`, which emits a
/// `rerun-if-env-changed` per option, so cargo rebuilds exactly what the new
/// value affects. (An earlier version had to wipe both crates before every
/// sweep point, which dominated the sweep's wall-clock cost.)
pub fn build(features: &str, env: &[(String, String)]) -> Result<()> {
    let sh = Shell::new()?;
    let env_desc = if env.is_empty() {
        "defaults".to_string()
    } else {
        env.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    eprintln!("=== building firmware (features: {features}; config: {env_desc}) ===");
    let mut c = cmd!(
        sh,
        "cargo build --release --target {TARGET} -p ssh-stamp-esp32 --bin ssh-stamp-esp32 --no-default-features --features {features}"
    );
    for (k, v) in env {
        c = c.env(k, v);
    }
    c.run().context("cargo build of firmware failed")?;
    Ok(())
}

/// Flashes the freshly-built ELF via espflash (no `--monitor`: xtask owns the
/// serial port).
///
/// `port` is passed through explicitly rather than left to espflash's own
/// detection. A typical bench rig exposes two ports — the SoC's console and the
/// USB-serial adapter wired to the bridged UART under test — and with more than
/// one candidate espflash prompts for a choice, which stalls an unattended
/// sweep. Reusing the port xtask already resolved also guarantees it flashes the
/// same device it is about to read checkpoints from.
pub fn flash(port: &str, baud: u32) -> Result<()> {
    let sh = Shell::new()?;
    let baud = baud.to_string();
    eprintln!("=== flashing {ELF} to {port} ===");
    cmd!(
        sh,
        "espflash flash --port {port} --baud {baud} --partition-table {PARTITIONS} --chip {CHIP} {ELF}"
    )
    .run()
    .context("espflash flash failed")?;
    Ok(())
}

/// Flashes the ELF and keeps espflash attached as the serial monitor, returning
/// the child with its stdout piped (flash progress stays on stderr).
///
/// This exists for output emitted in *early boot*: the USB-Serial-JTAG console
/// re-enumerates across the post-flash reset, and by the time [`flash`] exits
/// and xtask opens the port itself, the first second of boot output is gone
/// (which is why the boot checkpoints need a replay). espflash's own
/// flash→monitor handoff reattaches fast enough to show output from the ROM
/// bootloader onward, so callers that need boot-time lines — `xtask crypto` —
/// read espflash's stdout instead of the port. The caller kills the child once
/// it has seen what it needs.
pub fn flash_monitor(port: &str, baud: u32) -> Result<std::process::Child> {
    eprintln!("=== flashing {ELF} to {port} (monitored) ===");
    Command::new("espflash")
        .args(["flash", "--port", port, "--baud", &baud.to_string()])
        .args(["--partition-table", PARTITIONS, "--chip", CHIP])
        .args(["--monitor", "--non-interactive"])
        .arg(ELF)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn espflash flash --monitor")
}

/// Grace period after authentication, to catch a session that dies immediately
/// afterwards (e.g. the server refusing the shell request).
const AUTH_GRACE: Duration = Duration::from_millis(500);

/// Builds the `ssh` invocation shared by both session drivers.
///
/// `-T`, and **never** a remote command. The firmware fails every exec request
/// (`session_exec` in `src/handle.rs` calls `a.fail()` unconditionally), so
/// `ssh host <cmd>` is refused *after* a completely successful KEX and auth and
/// ssh exits 255 — scoring a perfectly good session as a drop. With no command
/// ssh asks for a shell, which the firmware answers by bridging the channel to
/// UART. No PTY is requested, because that bridge is a byte pipe, not a
/// terminal.
///
/// A throwaway `UserKnownHostsFile` keeps repeated runs from prompting or
/// polluting the user's file, and `BatchMode` fails fast instead of asking for a
/// password.
///
/// `extra_opts` are further `-o KEY=VALUE` settings, used to pin the negotiated
/// KEX algorithm for a crypto A/B. `verbose` adds `-v`, whose trace is how
/// [`ssh_session`] detects that authentication succeeded.
///
/// Every flag goes *before* the destination: OpenSSH stops parsing options at
/// the first non-option argument, so `ssh host -v` would ask the server to run a
/// command called `-v`.
fn ssh_session_cmd(
    host: &str,
    user: &str,
    connect_timeout_s: u32,
    extra_opts: &[String],
    verbose: bool,
) -> Command {
    let mut c = Command::new("ssh");
    c.arg("-T")
        .args(["-o", "BatchMode=yes"])
        .args(["-o", "StrictHostKeyChecking=no"])
        .args(["-o", &format!("UserKnownHostsFile={}", null_device())])
        .args(["-o", &format!("ConnectTimeout={connect_timeout_s}")]);
    if verbose {
        c.arg("-v");
    }
    for opt in extra_opts {
        c.args(["-o", opt]);
    }
    c.arg(format!("{user}@{host}"));
    c
}

/// Whether the host's `ssh` client offers `alg` as a key-exchange algorithm.
///
/// Checked up front because an unsupported name (`mlkem768x25519-sha256` on
/// OpenSSH older than 9.9) makes every session fail in exactly the same way an
/// unenrolled key does — a confusing way to find out the client is too old.
pub fn ssh_supports_kex(alg: &str) -> Result<bool> {
    let out = Command::new("ssh")
        .args(["-Q", "kex"])
        .output()
        .context("could not run the ssh client; is it on PATH?")?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.trim() == alg))
}

/// What one driven SSH session reported about itself.
pub struct SessionReport {
    /// KEX, auth and the session channel all succeeded.
    pub established: bool,
    /// The key-exchange algorithm the session negotiated, from ssh's own `-v`
    /// trace. Ground truth for what the KEX samples measured — present even
    /// when auth later failed, because KEX completes first.
    pub kex_algorithm: Option<String>,
}

/// The negotiated key-exchange algorithm, if `line` is the `-v` trace line that
/// names it (`debug1: kex: algorithm: mlkem768x25519-sha256`). The similar
/// `kex: host key algorithm:` line does not match — that names the host key.
fn parse_kex_algorithm(line: &str) -> Option<String> {
    let alg = line.split("kex: algorithm: ").nth(1)?.trim();
    (!alg.is_empty()).then(|| alg.to_string())
}

/// Opens one SSH session the way a user does — KEX, auth, session channel, UART
/// bridge — and reports whether it got that far. A failed session is counted,
/// not fatal; `Err` is reserved for a missing `ssh` client.
///
/// Success cannot be "exit status 0": the firmware never closes the bridged
/// channel on its own, so a healthy session simply stays open and has to be torn
/// down from this side. Nor can it be "still running after N seconds" — that
/// scores a session still grinding through TCP connect or KEX as a success.
///
/// So it watches what ssh itself reports: `-v` prints `Authenticated to <host>`
/// the instant auth succeeds, which is unambiguous, needs no `mem-probe`
/// firmware, and arrives as soon as it happens rather than after a fixed wait.
/// After that line the session is given [`AUTH_GRACE`] to prove it survives (a
/// refused shell request would kill it) and then closed.
///
/// stdin is an open pipe that is never written to, and **must not** be
/// `Stdio::null()`. A null stdin is at end-of-file the moment it is opened, so
/// ssh forwards `SSH_MSG_CHANNEL_EOF` as soon as the channel exists; the
/// firmware's bridge treats a zero-length read as the end of the session
/// (`ssh_to_uart` in `src/serial.rs` returns `ChannelEOF`), drops the socket, and
/// every session is scored as a drop with `Connection reset by peer` — a healthy
/// device failing an unhealthy measurement. Holding the pipe open for the life of
/// the child is what makes this look like the interactive session it is standing
/// in for.
pub fn ssh_session(
    host: &str,
    user: &str,
    connect_timeout_s: u32,
    extra_opts: &[String],
) -> Result<SessionReport> {
    let mut child = ssh_session_cmd(host, user, connect_timeout_s, extra_opts, true)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run the ssh client; is it on PATH?")?;

    let stderr = child.stderr.take().expect("stderr was piped");
    let authed = Arc::new(AtomicBool::new(false));
    let trace = Arc::new(Mutex::new(Vec::new()));
    let authed_t = Arc::clone(&authed);
    let trace_t = Arc::clone(&trace);
    let reader = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.contains("Authenticated to") {
                authed_t.store(true, Ordering::Relaxed);
            }
            trace_t.lock().expect("ssh trace poisoned").push(line);
        }
    });

    // Generous ceiling: ssh gives up on its own inside ConnectTimeout, so this
    // only bounds a client wedged for some other reason.
    let deadline = Instant::now() + Duration::from_secs(connect_timeout_s.into()) + AUTH_GRACE * 4;
    let mut result = false;
    loop {
        if let Some(status) = child.try_wait().context("waiting on the ssh client")? {
            // Exited on its own: clean means the peer closed the channel (fine),
            // non-zero means it never got in.
            result = status.success();
            break;
        }
        if authed.load(Ordering::Relaxed) {
            thread::sleep(AUTH_GRACE);
            // Authenticated. Still running means it is holding the bridged
            // channel; a clean exit means the peer closed it. Both are good
            // sessions — only a non-zero exit this soon after auth is not.
            result = child
                .try_wait()
                .context("waiting on the ssh client")?
                .is_none_or(|status| status.success());
            let _ = child.kill();
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = child.wait();
    let _ = reader.join();
    let trace = trace.lock().expect("ssh trace poisoned");
    if !result {
        report_ssh_failure(&trace);
    }
    Ok(SessionReport {
        established: result,
        kex_algorithm: trace.iter().find_map(|l| parse_kex_algorithm(l)),
    })
}

/// Unmeasured warm-up round trips before the measured ones — ARP resolution,
/// Wi-Fi power-save wake-up and allocator warm-up all land on the first writes.
const RTT_WARMUP: u32 = 3;

/// How long one marker gets to come back before it is scored as lost.
const RTT_MARKER_TIMEOUT: Duration = Duration::from_secs(2);

/// The `i`-th round-trip marker: 16 printable ASCII bytes with no 0x0A/0x0D, so
/// no line discipline on either side can rewrite it, and unique per iteration,
/// so a late echo of marker N can never satisfy the wait for marker N+1.
fn rtt_marker(i: u32) -> Vec<u8> {
    format!("[rtt:{i:010}]").into_bytes()
}

/// First offset of `needle` in `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Outcome of an [`ssh_rtt`] run.
pub struct RttOutcome {
    /// Host-clock round-trip times, in µs, one per marker that returned.
    pub samples_us: Vec<u64>,
    /// Markers that never came back within [`RTT_MARKER_TIMEOUT`].
    pub timeouts: u32,
    /// The negotiated KEX algorithm, as in [`SessionReport`].
    pub kex_algorithm: Option<String>,
}

/// Measures the bridge round trip over one interactive SSH session: each
/// marker is written to ssh's stdin and timed until it reappears on stdout,
/// having travelled Wi-Fi → SSH channel → UART TX → loopback → UART RX → back.
/// Requires firmware built with `bench-loopback`, which routes the bridged
/// UART's TX signal into its own RX through the GPIO matrix.
///
/// The clock is the host's. The firmware carries zero measurement code for
/// this: what is measured is the whole production path, exactly as a user's
/// keystroke would traverse it. Round trips run strictly one at a time, so
/// each sample is an idle-path latency, not a pipelined throughput figure.
///
/// `Err` means the session itself could not be established; a marker that is
/// lost mid-run only increments `timeouts` (an all-lost run is the caller's
/// signal that the loopback is not in the image).
pub fn ssh_rtt(
    host: &str,
    user: &str,
    connect_timeout_s: u32,
    extra_opts: &[String],
    iters: u32,
) -> Result<RttOutcome> {
    let mut child = ssh_session_cmd(host, user, connect_timeout_s, extra_opts, true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run the ssh client; is it on PATH?")?;

    // Auth detection, exactly as in `ssh_session`.
    let stderr = child.stderr.take().expect("stderr was piped");
    let authed = Arc::new(AtomicBool::new(false));
    let trace = Arc::new(Mutex::new(Vec::new()));
    let authed_t = Arc::clone(&authed);
    let trace_t = Arc::clone(&trace);
    let reader = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.contains("Authenticated to") {
                authed_t.store(true, Ordering::Relaxed);
            }
            trace_t.lock().expect("ssh trace poisoned").push(line);
        }
    });

    // Echoed bytes are handed over a channel so the measuring loop can wait
    // with a timeout instead of blocking on the pipe.
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let out_reader = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Nothing may be written before auth: earlier bytes would be buffered into
    // the very first round trip and be measured as bridge latency.
    let auth_deadline =
        Instant::now() + Duration::from_secs(connect_timeout_s.into()) + AUTH_GRACE * 4;
    while !authed.load(Ordering::Relaxed) {
        let exited = child
            .try_wait()
            .context("waiting on the ssh client")?
            .is_some();
        if exited || Instant::now() >= auth_deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            report_ssh_failure(&trace.lock().expect("ssh trace poisoned"));
            bail!("the RTT session never authenticated to {user}@{host}");
        }
        thread::sleep(Duration::from_millis(20));
    }

    let mut stdin = child.stdin.take().expect("stdin was piped");
    let mut samples_us = Vec::new();
    let mut timeouts = 0u32;
    // Bytes echoed back but not yet consumed by a marker match. A marker is
    // consumed up to and including itself, so a straggler arriving late sits
    // here harmlessly — it can never match a later marker's pattern.
    let mut pending: Vec<u8> = Vec::new();
    for i in 0..iters + RTT_WARMUP {
        let marker = rtt_marker(i);
        let start = Instant::now();
        if stdin
            .write_all(&marker)
            .and_then(|()| stdin.flush())
            .is_err()
        {
            // The session died mid-run; every remaining marker is lost.
            timeouts += (iters + RTT_WARMUP - i).min(iters);
            break;
        }
        let deadline = start + RTT_MARKER_TIMEOUT;
        let returned = loop {
            if let Some(pos) = find_subslice(&pending, &marker) {
                pending.drain(..pos + marker.len());
                break true;
            }
            let now = Instant::now();
            if now >= deadline {
                break false;
            }
            if let Ok(chunk) = rx.recv_timeout(deadline - now) {
                pending.extend_from_slice(&chunk);
            }
        };
        let elapsed = start.elapsed();
        if i < RTT_WARMUP {
            continue;
        }
        if returned {
            samples_us.push(elapsed.as_micros() as u64);
        } else {
            timeouts += 1;
        }
    }

    // Closing stdin is the polite teardown (the bridge reads EOF); the kill
    // covers a client that lingers anyway.
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    let _ = out_reader.join();
    let trace = trace.lock().expect("ssh trace poisoned");
    Ok(RttOutcome {
        samples_us,
        timeouts,
        kex_algorithm: trace.iter().find_map(|l| parse_kex_algorithm(l)),
    })
}

/// Lines in ssh's `-v` trace that name a real reason a bench session did not get
/// in. Ordered by nothing in particular — the *last* match wins, since ssh
/// narrates its way to the failure.
const SSH_FAILURE_MARKERS: &[&str] = &[
    "Permission denied",
    "Connection timed out",
    "Connection refused",
    "Connection reset",
    "No route to host",
    "Network is unreachable",
    "Unable to negotiate",
    "no matching",
    "kex_exchange_identification",
    "Host key verification failed",
    "Bad configuration",
    "Too many authentication failures",
];

/// Prints the one line of ssh's trace that explains a failed session.
///
/// Without it a `FAILED` is unattributable: "the key is not enrolled", "the host
/// is not on the AP" and "the client is too old for this `--kex`" all look
/// identical from the outside, and the trace that distinguishes them was being
/// read and discarded.
fn report_ssh_failure(trace: &[String]) {
    // Checked ahead of the marker scan, and not by adding a marker, because a
    // refused shell is always *followed* by the socket going away — so "last
    // match wins" would report `Connection reset by peer` and send the reader
    // looking at the network, when the device deliberately said no.
    if let Some(line) = trace
        .iter()
        .find(|l| l.contains("request failed on channel"))
    {
        eprintln!("           ssh: {}", line.trim());
        eprintln!(
            "           ssh: authentication succeeded but the device refused the session. \
             On an unprovisioned device that is expected: `first_login` accepts the \
             connection before any signature, then the shell is rejected because no \
             pubkey was ever checked. Enrol a key (step 4b) and re-run."
        );
        return;
    }

    let culprit = trace
        .iter()
        .rev()
        .find(|l| SSH_FAILURE_MARKERS.iter().any(|m| l.contains(m)))
        .or_else(|| trace.iter().rev().find(|l| !l.starts_with("debug")));
    let Some(line) = culprit else { return };
    eprintln!("           ssh: {}", line.trim());

    // A pubkey denial is the one failure the summary line cannot explain on its
    // own: the device holds exactly the key that `SSH_STAMP_PUBKEY` enrolled, so
    // what matters is which keys this client actually offered — and whether it
    // offered any at all. `BatchMode=yes` (which every session sets, to keep an
    // unattended run from blocking on a prompt) silently declines to unlock a
    // passphrase-protected key that no agent is holding, and that presents
    // identically to a key that was never enrolled.
    if !line.contains("Permission denied (publickey)") {
        return;
    }
    let offered: Vec<&String> = trace
        .iter()
        .filter(|l| l.contains("Offering public key:") || l.contains("Will attempt key:"))
        .collect();
    if offered.is_empty() {
        eprintln!(
            "           ssh: offered no keys at all — ssh found no usable identity. \
             Under BatchMode a passphrase-protected key needs an agent (`ssh-add -l`)."
        );
        return;
    }
    for l in offered {
        eprintln!("           ssh: {}", l.trim());
    }

    // Pubkey auth runs in two phases (RFC 4252 §7): the client first *queries*
    // whether a key would be acceptable, and only then signs. Which phase failed
    // is the whole diagnosis, and the client is the only side that can say
    // without a serial cable attached. ssh prints "Server accepts key" when the
    // query is answered with PK_OK — the device recognised the key and asked for
    // a signature — so a denial after that line is a signature-verification
    // failure. No such line means the offer was refused outright.
    let accepted = trace
        .iter()
        .rev()
        .find(|l| l.contains("Server accepts key:"));
    if let Some(l) = accepted {
        eprintln!("           ssh: {}", l.trim());

        // PK_OK proves the key is enrolled, so the denial came later — but from
        // which side? A device that refused the signature answers with a second
        // USERAUTH_FAILURE, which ssh reports by listing the remaining methods
        // again. Exactly one such line means the only failure was the opening
        // `none` probe, so no signed request was ever sent and ssh gave up on
        // its own. The query phase needs just the public half, which is why this
        // gets all the way to PK_OK before failing.
        let refusals = trace
            .iter()
            .filter(|l| l.contains("Authentications that can continue:"))
            .count();
        if refusals >= 2 {
            eprintln!(
                "           ssh: the device recognised this key and then refused \
                 its signature — a verification failure, not an enrolment one."
            );
        } else {
            eprintln!(
                "           ssh: the device recognised this key, but ssh never \
                 sent a signature — it could not use the private half. Under \
                 BatchMode (which every xtask session sets, so an unattended run \
                 cannot block on a prompt) a passphrase-protected key needs an \
                 agent; check `ssh-add -l`."
            );
        }
    } else {
        eprintln!(
            "           ssh: no offer was answered with PK_OK, so no key above \
             matched an enrolled slot. The device accepts only the key enrolled \
             via SSH_STAMP_PUBKEY — compare the fingerprints above with that \
             variable's contents."
        );
    }

    // The methods the device advertised in USERAUTH_FAILURE. If `publickey` is
    // absent the offers above were never going to be considered at all, which is
    // a server-side fault rather than a wrong key.
    if let Some(l) = trace
        .iter()
        .rev()
        .find(|l| l.contains("Authentications that can continue:"))
    {
        eprintln!("           ssh: {}", l.trim());
    }
}

/// The platform null device, used for a throwaway `UserKnownHostsFile`.
fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

/// Lists serial ports the OS currently exposes.
pub fn available_ports() -> Result<Vec<PathBuf>> {
    serial2::SerialPort::available_ports().context("enumerating serial ports")
}

/// Resolves the serial port to use: the explicit `--port`, or the sole
/// available port when exactly one is present, else a helpful error listing
/// the candidates.
pub fn resolve_port(explicit: Option<&str>) -> Result<String> {
    if let Some(p) = explicit {
        return Ok(p.to_string());
    }
    let ports = available_ports().unwrap_or_default();
    match ports.as_slice() {
        [only] => {
            let name = only.display().to_string();
            eprintln!("auto-selected serial port {name}");
            Ok(name)
        }
        [] => bail!("no serial ports found; pass --port (espflash prints it while flashing)"),
        many => {
            let list = many
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("multiple serial ports found ({list}); pass --port to choose one")
        }
    }
}

/// Serial read timeout — short, so the reader notices `stop` and lost ports
/// promptly.
const SERIAL_READ_TIMEOUT: Duration = Duration::from_millis(200);

/// Pause before trying to reopen a port that went away, so a permanently
/// missing device does not spin.
const SERIAL_REOPEN_DELAY: Duration = Duration::from_millis(250);

/// Opens a port for capture (8N1, non-blocking-ish via a read timeout).
fn open_port(port: &str, baud: u32) -> Result<serial2::SerialPort> {
    let mut sp = serial2::SerialPort::open(port, baud)
        .with_context(|| format!("opening serial port {port}"))?;
    sp.set_read_timeout(SERIAL_READ_TIMEOUT)
        .context("setting serial read timeout")?;
    Ok(sp)
}

/// A background serial reader: opens the port, accumulates complete lines in a
/// shared buffer, and (optionally) echoes them. Stops the reader thread on
/// drop.
pub struct Serial {
    port: String,
    lines: Arc<Mutex<Vec<String>>>,
    error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Serial {
    /// Opens `port` at `baud` (8N1) and starts collecting lines. With `echo`,
    /// each line is also printed to stdout so the user sees the device boot.
    ///
    /// The reader reopens the port if it disappears mid-capture, and remembers
    /// the last read error for [`Serial::last_error`]. Both matter on a SoC
    /// whose console is native USB-Serial-JTAG (the C6's): the port *is* part of
    /// the chip, so every reset re-enumerates it — the OS tears the device node
    /// down and builds a new one. A handle opened just as espflash resets the
    /// chip therefore refers to a node that is already dying, and every read on
    /// it fails. Treating that as end-of-stream is what turns a working rig into
    /// a run with no checkpoints and no explanation.
    pub fn open(port: &str, baud: u32, echo: bool) -> Result<Serial> {
        // Opened eagerly so a bad `--port` is reported as such, here, instead of
        // as an unexplained absence of `@BENCH` lines two minutes later.
        let sp = open_port(port, baud)?;

        let lines = Arc::new(Mutex::new(Vec::new()));
        let error = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let lines_t = Arc::clone(&lines);
        let error_t = Arc::clone(&error);
        let stop_t = Arc::clone(&stop);
        let name = port.to_string();

        let handle = thread::spawn(move || {
            let mut sp = Some(sp);
            let mut buf = [0u8; 1024];
            let mut partial = String::new();
            while !stop_t.load(Ordering::Relaxed) {
                let Some(open) = sp.as_mut() else {
                    thread::sleep(SERIAL_REOPEN_DELAY);
                    sp = open_port(&name, baud).ok();
                    continue;
                };
                match open.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        partial.push_str(&String::from_utf8_lossy(&buf[..n]));
                        while let Some(pos) = partial.find('\n') {
                            let raw: String = partial.drain(..=pos).collect();
                            let line = raw.trim_end().to_string();
                            if echo {
                                println!("{line}");
                            }
                            lines_t.lock().expect("serial buffer poisoned").push(line);
                        }
                    }
                    Err(e)
                        if e.kind() == io::ErrorKind::TimedOut
                            || e.kind() == io::ErrorKind::Interrupted
                            || e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => {
                        *error_t.lock().expect("serial error slot poisoned") = Some(e.to_string());
                        sp = None;
                    }
                }
            }
        });

        Ok(Serial {
            port: port.to_string(),
            lines,
            error,
            stop,
            handle: Some(handle),
        })
    }

    /// Returns a copy of every complete line seen so far.
    pub fn snapshot(&self) -> Vec<String> {
        self.lines.lock().expect("serial buffer poisoned").clone()
    }

    /// True if any captured line contains `needle`.
    pub fn contains(&self, needle: &str) -> bool {
        self.lines
            .lock()
            .expect("serial buffer poisoned")
            .iter()
            .any(|l| l.contains(needle))
    }

    /// The most recent read error, if the port has failed at any point.
    pub fn last_error(&self) -> Option<String> {
        self.error
            .lock()
            .expect("serial error slot poisoned")
            .clone()
    }

    /// Warns on stderr if the capture looks broken rather than merely quiet,
    /// naming the cause when it is knowable. Silence here is otherwise
    /// indistinguishable from a firmware built without `mem-probe`.
    pub fn report_health(&self) {
        let captured = self.lines.lock().expect("serial buffer poisoned").len();
        let Some(err) = self.last_error() else {
            if captured == 0 {
                eprintln!(
                    "note: nothing was read from {port}. An idle device that has not been \
                     reset is legitimately silent, so this is only a problem if output was \
                     expected — in which case check that no other program holds the port (an \
                     espflash monitor left open, a terminal) and that {port} is the SoC's \
                     console rather than the adapter wired to the bridged UART.",
                    port = self.port
                );
            }
            return;
        };
        eprintln!(
            "warning: reading {port} failed ({err}); captured {captured} line(s). \
             A console on native USB-Serial-JTAG re-enumerates on every chip reset, \
             so a handle taken during the post-flash reset dies with the old device node.",
            port = self.port
        );
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

/// How the device announced that it was ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ready {
    /// The `bench_tcp_listening` checkpoint arrived on serial.
    Checkpoint,
    /// The checkpoint never arrived, but `host:22` accepted a connection. The
    /// device is up; the *capture* is not telling us about it.
    TcpOnly,
}

/// Waits until the device is ready to accept SSH — either the
/// `bench_tcp_listening` checkpoint appears on serial, or a TCP connection to
/// `host:22` succeeds — within `timeout`.
///
/// The two are reported separately rather than folded into a bool. The TCP
/// fallback exists so a firmware without `mem-probe` still benches, but it also
/// happily papers over a capture that is producing nothing at all, which then
/// surfaces much later as an empty results table.
///
/// `require_checkpoint` drops the fallback and insists on the serial line. That
/// turns this into a deliberate wait for a *fresh boot*: an already-running
/// device answers on port 22 immediately, so with the fallback in play there is
/// no window in which a hand-pressed reset could be observed, and the one-shot
/// startup checkpoints can never be recorded.
pub fn wait_for_ready(
    serial: &Serial,
    host: &str,
    timeout: Duration,
    require_checkpoint: bool,
) -> Option<Ready> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if serial.contains(READY_CHECKPOINT) {
            return Some(Ready::Checkpoint);
        }
        if !require_checkpoint && tcp_port_open(host, 22) {
            return Some(Ready::TcpOnly);
        }
        thread::sleep(Duration::from_millis(250));
    }
    None
}

/// Asks the OS to associate this host with `ssid`, using whichever profile it
/// already has saved. No credentials pass through xtask: join the network once
/// by hand and every later re-join is automatic.
fn join_ap(ssid: &str) -> Result<()> {
    if cfg!(windows) {
        // `name` selects the saved profile; `ssid=` is deliberately not passed,
        // as supplying it makes netsh require `interface=` even where it would
        // otherwise be optional.
        let interfaces = wlan_interfaces();
        // No `interface=` at all is right on a single-adapter machine, and is
        // the only thing left when the adapter list could not be read.
        let attempts: Vec<Option<&str>> = if interfaces.is_empty() {
            vec![None]
        } else {
            interfaces.iter().map(|i| Some(i.as_str())).collect()
        };
        let mut failures = Vec::new();
        for interface in attempts {
            let mut c = Command::new("netsh");
            c.args(["wlan", "connect"]).arg(format!("name={ssid}"));
            if let Some(i) = interface {
                c.arg(format!("interface={i}"));
            }
            match run_join(&mut c) {
                Ok(()) => return Ok(()),
                Err(e) => failures.push(match interface {
                    Some(i) => format!("{i}: {e}"),
                    None => e.to_string(),
                }),
            }
        }
        bail!("{}", failures.join("; "))
    }
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("networksetup");
        c.args(["-setairportnetwork", "en0", ssid]);
        c
    } else {
        let mut c = Command::new("nmcli");
        c.args(["device", "wifi", "connect", ssid]);
        c
    };
    run_join(&mut cmd)
}

/// The wireless adapters `netsh` reports, in the order it lists them.
///
/// `netsh wlan connect` requires `interface=` as soon as the system has more
/// than one wireless adapter, and refuses the command with the same "One or more
/// parameters for the command are not correct or missing" it uses for a genuine
/// syntax error — so an ambiguous machine is indistinguishable from a malformed
/// command line, and the rig looks broken. A Wi-Fi Direct virtual adapter is
/// enough to trip it, and those come and go with Mobile hotspot and Miracast, so
/// it cannot be settled once during rig setup. Naming an adapter every time
/// sidesteps the whole distinction.
///
/// Empty on a non-English Windows, whose field labels are localised, and on any
/// other host — where `netsh` is absent and the caller does not use this. An
/// empty list means "attempt without `interface=`", which is what this did
/// before and still works wherever the ambiguity does not arise.
fn wlan_interfaces() -> Vec<String> {
    let Ok(out) = Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once(':'))
        .filter(|(k, _)| k.trim() == "Name")
        .map(|(_, v)| v.trim().to_string())
        .collect()
}

/// Runs one join attempt, reducing a non-zero exit to the tool's own first line
/// of complaint. These tools report failure on stdout as often as stderr.
fn run_join(cmd: &mut Command) -> Result<()> {
    let out = cmd
        .output()
        .with_context(|| format!("running {:?}", cmd.get_program()))?;
    if out.status.success() {
        return Ok(());
    }
    let msg: String = String::from_utf8_lossy(&out.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&out.stderr).lines())
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no output")
        .to_string();
    bail!("{msg}")
}

/// How often to re-issue the join while waiting. Long enough for an association
/// plus DHCP to complete before the next attempt disturbs it.
const REJOIN_INTERVAL: Duration = Duration::from_secs(8);

/// Waits until `host:22` accepts a TCP connection — that is, until *this* host
/// can actually reach the device — re-joining `ap_ssid` while it waits.
///
/// Deliberately separate from [`wait_for_ready`], which reports that the
/// firmware is listening and learns it from the device's own serial output. The
/// two are not the same thing on an AP-mode rig: flashing resets the SoC, the
/// access point goes down with it, and the host's supplicant re-joins on its own
/// schedule — seconds to tens of seconds, and often via some other remembered
/// network first. The device announces `bench_tcp_listening` at ~2.7 s
/// regardless, so acting on that alone drives every session into a routing black
/// hole and scores a healthy device as a total failure.
///
/// Waiting alone is not enough on Windows, which falls back to whichever
/// remembered network has internet and then, being connected, never scans for
/// the one that came back. Every sweep point flashes, so an unattended run would
/// stall on a healthy device once per point. Asking for the join explicitly is
/// what makes a multi-point sweep possible without a human at the Wi-Fi menu.
pub fn wait_for_reachable(host: &str, timeout: Duration, ap_ssid: Option<&str>) -> bool {
    let deadline = Instant::now() + timeout;
    let mut next_join = Instant::now();
    // Attempts continue after a failure — an adapter that is mid-scan or mid-DHCP
    // refuses one join and takes the next — but only the first complaint is
    // printed. A missing `nmcli` fails identically every time, and saying so once
    // is help where saying so eight times is noise.
    let mut complained = false;
    loop {
        if tcp_port_open(host, 22) {
            return true;
        }
        if let Some(ssid) = ap_ssid
            && Instant::now() >= next_join
        {
            next_join = Instant::now() + REJOIN_INTERVAL;
            match join_ap(ssid) {
                Ok(()) => eprintln!("=== asked this host to join {ssid} ==="),
                Err(e) if !complained => {
                    complained = true;
                    eprintln!("warning: could not join {ssid} ({e}); join it by hand");
                }
                Err(_) => {}
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(500));
    }
}

/// Cheap TCP-port liveness probe.
fn tcp_port_open(host: &str, port: u16) -> bool {
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs.any(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok())
}

/// Aggregate result of a load run (`xtask sweep`'s load generator).
#[derive(Debug, Clone, Copy)]
pub struct LoadStats {
    /// Total bytes pushed host→device across all sessions.
    pub bytes_sent: u64,
    /// Host→device send throughput, KiB/s (an RX-pressure proxy).
    pub throughput_kib_s: f64,
    /// Sessions attempted.
    pub sessions: u32,
    /// Sessions that failed to establish (ssh exit 255) — a drop proxy.
    pub failures: u32,
}

/// Drives `concurrency` parallel SSH sessions against the device for `duration`,
/// each pushing `payload` through the channel (which the firmware bridges to
/// UART). This exercises the Wi-Fi → TCP → SSH receive path and creates the RX
/// pressure the buffer sweep measures. Returns aggregate host→device send
/// throughput plus a failed-session count.
///
/// Throughput here is *send* throughput observed from the host: bytes accepted
/// into the SSH channel per second. When the device can't keep up, the channel
/// window backs up, the writes block, and throughput drops — which is exactly
/// the degradation signal the sweep looks for.
pub fn run_load(
    host: &str,
    user: &str,
    connect_timeout_s: u32,
    concurrency: u32,
    payload: &[u8],
    duration: Duration,
) -> LoadStats {
    let start = Instant::now();
    let deadline = start + duration;
    let payload = Arc::new(payload.to_vec());
    let host = host.to_string();
    let user = user.to_string();

    let handles: Vec<_> = (0..concurrency.max(1))
        .map(|_| {
            let host = host.clone();
            let user = user.clone();
            let payload = Arc::clone(&payload);
            thread::spawn(move || {
                let (mut bytes, mut sessions, mut failures) = (0u64, 0u32, 0u32);
                while Instant::now() < deadline {
                    let out = one_load_session(&host, &user, connect_timeout_s, &payload, deadline);
                    bytes += out.bytes_sent;
                    sessions += 1;
                    if !out.established {
                        failures += 1;
                    }
                }
                (bytes, sessions, failures)
            })
        })
        .collect();

    let (mut bytes_sent, mut sessions, mut failures) = (0u64, 0u32, 0u32);
    for h in handles {
        if let Ok((b, s, f)) = h.join() {
            bytes_sent += b;
            sessions += s;
            failures += f;
        }
    }
    let elapsed_s = start.elapsed().as_secs_f64();
    let throughput_kib_s = if elapsed_s > 0.0 {
        (bytes_sent as f64 / 1024.0) / elapsed_s
    } else {
        0.0
    };
    LoadStats {
        bytes_sent,
        throughput_kib_s,
        sessions,
        failures,
    }
}

/// Outcome of a single load session.
struct SessionOutcome {
    bytes_sent: u64,
    established: bool,
}

/// Opens one non-interactive SSH session (`-T`, no remote command → the firmware
/// bridges the channel to UART), streams `payload` into its stdin, and kills it
/// at `deadline`. Returns how many bytes were accepted and whether the
/// connection established (ssh exit 255 = connect/auth failure).
fn one_load_session(
    host: &str,
    user: &str,
    connect_timeout_s: u32,
    payload: &[u8],
    deadline: Instant,
) -> SessionOutcome {
    let spawn = ssh_session_cmd(host, user, connect_timeout_s, &[], false)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawn {
        Ok(c) => c,
        // ssh missing / spawn failure: count as a non-established session rather
        // than aborting the whole sweep.
        Err(_) => {
            return SessionOutcome {
                bytes_sent: 0,
                established: false,
            };
        }
    };

    // Stream the payload from a dedicated thread so a full channel window (device
    // backpressure) blocks only the writer, not the deadline watchdog below.
    let stdin = child.stdin.take();
    let payload = payload.to_vec();
    let writer = thread::spawn(move || {
        let mut sent = 0u64;
        if let Some(mut si) = stdin {
            for chunk in payload.chunks(4096) {
                if si.write_all(chunk).is_err() {
                    break;
                }
                sent += chunk.len() as u64;
            }
            let _ = si.flush();
            // Dropping `si` closes stdin, signalling EOF to ssh.
        }
        sent
    });

    let mut established = true;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.code() == Some(255) {
                    established = false;
                }
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                established = false;
                break;
            }
        }
    }

    let bytes_sent = writer.join().unwrap_or(0);
    SessionOutcome {
        bytes_sent,
        established,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_negotiated_kex_algorithm() {
        assert_eq!(
            parse_kex_algorithm("debug1: kex: algorithm: mlkem768x25519-sha256"),
            Some("mlkem768x25519-sha256".into())
        );
        // The host-key line names the host key, not the KEX — it must not match.
        assert_eq!(
            parse_kex_algorithm("debug1: kex: host key algorithm: ssh-ed25519"),
            None
        );
        assert_eq!(parse_kex_algorithm("debug1: Authenticated to host"), None);
    }

    #[test]
    fn rtt_markers_are_fixed_width_pty_safe_and_unique() {
        let m = rtt_marker(7);
        assert_eq!(m.len(), 16);
        assert!(!m.contains(&b'\n') && !m.contains(&b'\r'));
        assert_ne!(rtt_marker(1), rtt_marker(2));
        assert_eq!(find_subslice(b"noise[rtt:0000000007]tail", &m), Some(5));
        assert_eq!(find_subslice(&m, b"[rtt:9"), None);
    }
}
