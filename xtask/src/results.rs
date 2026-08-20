// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! This file contains the result records outputted by the benchmarks.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The main result that gets outputted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Results {
    /// Output of `xtask bench`.
    Bench(BenchResults),
    /// Output of `xtask size`.
    Size(SizeResults),
}

impl Results {
    /// Serializes to a JSON at `path`.
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
            .with_context(|| format!("parsing {} as a result", path.display()))
    }
}

/// The results from `xtask bench`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResults {
    /// The features used for the bench.
    pub features: String,
    /// The bench runs.
    pub runs: Vec<RunOutcome>,
}

impl BenchResults {
    /// The completed bench run.
    pub fn default_run(&self) -> Option<&BenchRun> {
        self.runs
            .iter()
            .find(|r| r.heap_size().is_none())
            .and_then(|r| r.bench())
    }
}

/// The outcome of the run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum RunOutcome {
    /// The bench failed before producing a result.
    Failed {
        /// The size of the heap.
        heap_size: Option<u64>,
        /// Whether the firmware reached the ready checkpoint.
        ready: bool,
    },
    /// The bench run if it was successful.
    Ok {
        /// The size of the heap.
        heap_size: Option<u64>,
        /// The benchmarking run.
        #[serde(flatten)]
        run: BenchRun,
    },
}

impl RunOutcome {
    /// The size of the heap.
    pub fn heap_size(&self) -> Option<u64> {
        match self {
            RunOutcome::Failed { heap_size, .. } | RunOutcome::Ok { heap_size, .. } => *heap_size,
        }
    }

    /// The completed bench.
    pub fn bench(&self) -> Option<&BenchRun> {
        match self {
            RunOutcome::Ok { run, .. } => Some(run),
            RunOutcome::Failed { .. } => None,
        }
    }
}

/// The measurements from a benchmarking run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRun {
    /// The KEX algorithm used for the session.
    pub kex_algorithm: String,
    /// The boot checkpoints.
    #[serde(default)]
    pub boot: Vec<BootCheckpoint>,
    /// The heap snapshots.
    #[serde(default)]
    pub heap: Vec<HeapSnapshot>,
    /// The max stack snapshots.
    #[serde(default)]
    pub stack: Vec<StackSnapshot>,
    /// The KEX time measurements in microseconds
    #[serde(default)]
    pub kex_us: Vec<u64>,
    /// The round-trip time measurements in microseconds.
    #[serde(default)]
    pub rtt_us: Vec<u64>,
}

impl BenchRun {
    /// The absolute timestamp of a checkpoint
    pub fn boot_t(&self, label: &str) -> Option<u64> {
        self.boot
            .iter()
            .find(|b| b.label == label)
            .map(|b| b.t_abs_us)
    }

    /// The maximum stack usage of any snapshot.
    pub fn stack_max(&self) -> Option<&StackSnapshot> {
        self.stack.iter().max_by_key(|s| s.max_bytes)
    }
}

/// The boot checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootCheckpoint {
    /// The label of the boot checkpoint.
    pub label: String,
    /// The absolute time of the measurement.
    pub t_abs_us: u64,
}

/// The heap usage snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeapSnapshot {
    /// The label for the snapshot.
    pub label: String,
    /// The bytes used.
    pub used_bytes: u64,
    /// The total bytes.
    pub total_bytes: u64,
    /// The maximum bytes overall.
    pub max_bytes: u64,
}

/// The maximum stack usage snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackSnapshot {
    /// The label for the snapshot.
    pub label: String,
    /// The maximum bytes of the stack.
    pub max_bytes: u64,
    /// The reserved bytes for the stack.
    pub reserved_bytes: u64,
}

/// The firmware size results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizeResults {
    /// The size of the firmware entries.
    pub entries: Vec<SocSize>,
}

/// The size for a single firmware SoC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocSize {
    /// The SoC name.
    pub soc: String,
    /// The board that it was built for.
    pub board: String,
    /// The build profile.
    pub profile: String,
    /// The build target.
    pub target: String,
    /// The features used to build.
    pub features: String,
    /// The flash footprint.
    pub flash_bytes: u64,
    /// The internal RAM size.
    pub ram_bytes: u64,
    /// The stack reserved bytes.
    pub stack_reserved_bytes: u64,
    /// The crate contributions from `cargo bloat --crates`.
    #[serde(default)]
    pub crates: Vec<CrateSize>,
}

/// The crate's contribution from `cargo bloat --crates`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrateSize {
    /// The name of the crate.
    pub name: String,
    /// The size of the crate.
    #[serde(alias = "size")]
    pub size_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_outcome() {
        let failed = RunOutcome::Failed {
            heap_size: Some(49_152),
            ready: false,
        };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "heap_size": 49_152,
                "outcome": "failed",
                "ready": false,
            })
        );

        let back: RunOutcome = serde_json::from_value(json).unwrap();
        assert_eq!(back.heap_size(), Some(49_152));
        assert!(back.bench().is_none());

        let ok = RunOutcome::Ok {
            heap_size: None,
            run: BenchRun {
                kex_algorithm: "curve25519-sha256".into(),
                boot: vec![],
                heap: vec![],
                stack: vec![],
                kex_us: vec![285_000],
                rtt_us: vec![],
            },
        };
        let json = serde_json::to_value(&ok).unwrap();
        assert_eq!(json["outcome"], "ok");
        assert_eq!(json["kex_us"], serde_json::json!([285_000]));

        let back: RunOutcome = serde_json::from_value(json).unwrap();
        assert_eq!(back.bench().unwrap().kex_us, vec![285_000]);
    }
}
