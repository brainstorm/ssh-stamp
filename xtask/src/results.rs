// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Serializable result records. Each subcommand writes one tagged JSON file;
//! `xtask report` renders the markdown summary from several and `xtask bmf`
//! converts them for Bencher. JSON is the single ingestion surface.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Top-level tagged result, distinguished by `kind` in the JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Results {
    /// Output of `xtask bench` for one firmware variant: the boot→session
    /// timeline plus the per-session KEX samples.
    Bench(BenchResults),
    /// Output of `xtask size` across one or more SoCs.
    Size(SizeResults),
    /// Output of `xtask stack` across one or more SoCs.
    Stack(StackResults),
    /// Output of `xtask sweep`: a heap/buffer parameter sweep.
    Sweep(SweepResults),
}

impl Results {
    /// Serializes to pretty JSON at `path`.
    pub fn write(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
        eprintln!("wrote {}", path.display());
        Ok(())
    }

    /// Reads and deserializes a result file.
    pub fn read(path: &Path) -> Result<Results> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("parsing {} as an xtask results file", path.display()))
    }
}

/// One firmware variant's measurements, reconstructed from `@BENCH` serial lines
/// (no CPU halt). `xtask bench` captures the timeline and the KEX samples in a
/// single run — they come from the same sessions, so splitting them across two
/// subcommands only meant flashing twice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResults {
    /// The firmware build variant (`default` / `nomlkem`) — what the image is.
    pub variant: String,
    /// The key-exchange algorithm the sessions actually negotiated, from the
    /// ssh client's own `-v` trace — what the KEX samples measured, as opposed
    /// to what was built. `None` when no session got far enough to agree on
    /// one. This, not the variant, keys the `kex/…` Bencher metric.
    pub kex_algorithm: Option<String>,
    pub host: String,
    pub user: String,
    /// SSH sessions driven (1 for an `--rtt` run).
    pub sessions_driven: u32,
    /// Sessions that failed to establish — or, on an `--rtt` run, markers that
    /// never came back.
    pub failures: u32,
    /// One-shot startup checkpoints (`bench_boot` → `bench_tcp_listening`),
    /// each with its absolute µs timestamp since the monotonic clock came up.
    pub boot: Vec<BootCheckpoint>,
    /// Per-session checkpoint sets (`bench_tcp_accept` → `bench_channel_open`).
    pub sessions: Vec<Session>,
    /// Heap snapshots by label (`boot` / `peripherals` / `wifi_up`).
    pub heap: Vec<HeapSnapshot>,
    /// Raw `accept->firstauth` KEX wall-time samples, in microseconds.
    pub kex_us: Vec<u64>,
    /// Host-measured bridge round-trip samples from `bench --rtt`, in
    /// microseconds: ssh stdin → Wi-Fi → SSH channel → UART TX → loopback →
    /// UART RX → ssh stdout. Empty unless the firmware was built with
    /// `bench-loopback` and `--rtt` drove it.
    pub rtt_us: Vec<u64>,
}

impl BenchResults {
    /// Absolute timestamp of a named startup checkpoint.
    pub fn boot_t(&self, name: &str) -> Option<u64> {
        self.boot
            .iter()
            .find(|b| b.name == name)
            .map(|b| b.t_abs_us)
    }

    /// A heap snapshot by label.
    pub fn heap_at(&self, label: &str) -> Option<&HeapSnapshot> {
        self.heap.iter().find(|h| h.label == label)
    }
}

/// A startup checkpoint that fires once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootCheckpoint {
    pub name: String,
    pub t_abs_us: u64,
}

/// The per-session checkpoints captured for one SSH connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// `(checkpoint name, absolute µs)` in firmware emission order.
    pub checkpoints: Vec<(String, u64)>,
}

impl Session {
    /// Looks up the absolute timestamp of a named checkpoint in this session.
    pub fn t(&self, name: &str) -> Option<u64> {
        self.checkpoints
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| *t)
    }
}

/// A heap usage snapshot at a labelled point in the timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeapSnapshot {
    pub label: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
    /// High-water mark from esp-alloc's `internal-heap-stats`.
    pub max_bytes: u64,
}

/// Firmware size across the measured SoCs (`xtask size`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizeResults {
    /// One entry per SoC measured in this run.
    pub entries: Vec<SocSize>,
    /// The `[target.*]` keys in `.cargo/config.toml` that enforce
    /// `-C overflow-checks=on`. Empty means milestone (c)'s safety invariant is
    /// unguarded and the size gate fails.
    pub overflow_checks_enforced_by: Vec<String>,
}

/// Size figures for one SoC/profile build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocSize {
    pub soc: String,
    pub profile: String,
    pub target: String,
    pub features: String,
    /// Flash footprint = `.text` + `.rodata`.
    pub flash_b: u64,
    /// Static RAM footprint = `.data` + `.bss`.
    pub ram_b: u64,
    /// Every ELF section, so a reviewer can audit the flash/RAM figures and see
    /// the ESP-specific sections (`.rwtext`, `.dram0.*`, …) the headline
    /// two-section definition intentionally leaves out.
    pub sections: Vec<Section>,
    /// Per-crate `.text` contribution from `cargo bloat --crates`, largest
    /// first. Empty if `cargo bloat` was unavailable (it is a supplement).
    pub crates: Vec<CrateSize>,
}

/// One ELF section and its size in bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub name: String,
    pub size_b: u64,
}

/// A crate's `.text` contribution, from `cargo bloat --crates`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrateSize {
    pub name: String,
    pub size_b: u64,
}

/// Static per-function stack frames across the measured SoCs (`xtask stack`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackResults {
    /// One entry per SoC measured in this run.
    pub entries: Vec<SocStack>,
}

/// Static stack usage for one SoC's analysis build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocStack {
    pub soc: String,
    pub profile: String,
    pub target: String,
    /// Per-function static frame sizes, largest first (may be truncated to the
    /// caller's `--top`). These are *individual* frames, not a call-graph total.
    pub functions: Vec<StackFrame>,
    /// Number of functions the `.stack_sizes` section reported (before any
    /// `--top` truncation).
    pub total_functions: usize,
    /// The single largest static frame.
    pub max_frame_b: u64,
}

/// A single function's static stack frame size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackFrame {
    pub function: String,
    pub size_b: u64,
}

/// Output of `xtask sweep`: how heap and buffer sizes trade against throughput.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepResults {
    pub host: String,
    pub user: String,
    /// Firmware feature string every point was built with.
    pub features: String,
    /// Load profile applied at each point (recorded for reproducibility).
    pub load: SweepLoad,
    /// The knob being swept, e.g. `heap_size`.
    pub knob: String,
    /// `true` if this was a `--bisect` run (only a log2(N) subset of the axis
    /// was probed) rather than every value.
    pub bisect: bool,
    /// One row per value actually built and measured, ascending.
    pub points: Vec<SweepPoint>,
    /// Degradation threshold: a point is healthy if its throughput stays within
    /// this fraction of the best observed.
    pub tolerance: f64,
    /// Smallest healthy value — the recommended setting. `None` if nothing
    /// booted cleanly.
    pub recommended: Option<u64>,
}

/// The load profile applied at each sweep point.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SweepLoad {
    pub concurrency: u32,
    pub payload_kib: u32,
    pub duration_s: u32,
}

/// One measured sweep point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepPoint {
    /// The swept knob's value for this point.
    pub value: u64,
    /// Static RAM (`.data`+`.bss`) of the image actually built for this point,
    /// read from its ELF. A measurement, not an estimate.
    pub ram_b: u64,
    /// Heap high-water mark (`@BENCH heap … max_bytes`), or `None` if the board
    /// never reported one (e.g. it failed to boot).
    pub heap_max_b: Option<u64>,
    /// Whether the firmware reached the SSH-ready checkpoint.
    pub ready: bool,
    /// Suspected out-of-memory / instability: never became ready, or a panic /
    /// allocation-failure line appeared on serial.
    pub oom: bool,
    /// Host→device send throughput under load, in KiB/s (an RX-pressure proxy).
    pub throughput_kib_s: f64,
    /// Total bytes pushed through the SSH channel(s) during the load window.
    pub bytes_sent: u64,
    /// SSH sessions that failed to establish (a drop / instability proxy).
    pub failures: u32,
}

impl SweepPoint {
    /// A point is healthy if it booted, did not OOM, dropped no sessions, and
    /// held throughput within `tolerance` of `reference`.
    pub fn healthy(&self, reference: f64, tolerance: f64) -> bool {
        self.ready
            && !self.oom
            && self.failures == 0
            && self.throughput_kib_s >= tolerance * reference
    }
}
