// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask bmf` converts the collected `results.json` into the Bencher Metric Format.

use crate::cmd;
use crate::results::{BenchRun, Results, SocSize, StackSnapshot};
use crate::stats::Stats;
use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// The bencher measurement fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Measure {
    FlashBytes,
    HeapBytes,
    LatencyUs,
    RamBytes,
    StackBytes,
}

#[derive(ClapArgs)]
pub struct Args {
    /// Results JSON to convert.
    #[arg(long = "input", required = true)]
    inputs: Vec<PathBuf>,
    /// Write the BMF JSON output to this path.
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
}

pub fn run(args: &Args) -> Result<()> {
    let results = args
        .inputs
        .iter()
        .map(|p| Results::read(p))
        .collect::<Result<Vec<_>>>()?;

    let bmf = to_bmf(&results);
    if bmf.is_empty() {
        bail!("no metrics found in the input files");
    }

    let json = serde_json::to_string_pretty(&bmf)?;
    match &args.output {
        Some(out) => {
            fs::write(out, &json).with_context(|| format!("writing {}", out.display()))?;
            eprintln!("wrote {} benchmarks to {}", bmf.len(), out.display());
        }
        None => println!("{json}"),
    }
    Ok(())
}

/// One BMF metric.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Metric {
    value: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    lower_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upper_value: Option<f64>,
}

impl Metric {
    /// Construct using a single point with no range.
    fn point(value: f64) -> Self {
        Self {
            value,
            lower_value: None,
            upper_value: None,
        }
    }

    /// Construct using a value and a range.
    fn range(value: f64, lower: f64, upper: f64) -> Self {
        Self {
            value,
            lower_value: Some(lower),
            upper_value: Some(upper),
        }
    }
}

/// The BMF document.
type Bmf = BTreeMap<String, BTreeMap<Measure, Metric>>;

/// Inserts a `metric` under `bench` and `measure`. A duplicate only keeps the last value.
fn insert(bmf: &mut Bmf, bench: impl Into<String>, measure: Measure, metric: Metric) {
    bmf.entry(bench.into()).or_default().insert(measure, metric);
}

/// Converts every result into the BMF document.
fn to_bmf(results: &[Results]) -> Bmf {
    let mut bmf = Bmf::new();
    let mut runs: Vec<&BenchRun> = Vec::new();
    for result in results {
        match result {
            // Keep only the default run as `--heap` represents a probe only, not a bencer run.
            Results::Bench(bench) => runs.extend(bench.default_run()),
            Results::Size(size) => size.entries.iter().for_each(|e| size_metrics(&mut bmf, e)),
        }
    }

    for run in &runs {
        kex_metrics(&mut bmf, run);
    }
    if let Some(run) = reference_run(&runs) {
        device_metrics(&mut bmf, run);
    }

    if let Some(stack) = maximum_stack(&runs) {
        insert(
            &mut bmf,
            "stack/max",
            Measure::StackBytes,
            Metric::point(stack.max_bytes as f64),
        );
    }

    bmf
}

/// The run whose measurements stand for the device.
fn reference_run<'a>(runs: &[&'a BenchRun]) -> Option<&'a BenchRun> {
    runs.iter()
        .find(|run| run.kex_algorithm == cmd::REFERENCE_KEX)
        .or(runs.first())
        .copied()
}

/// The maximum stack size of any run.
fn maximum_stack<'a>(runs: &[&'a BenchRun]) -> Option<&'a StackSnapshot> {
    runs.iter()
        .filter_map(|run| run.stack_max())
        .max_by_key(|stack| stack.max_bytes)
}

/// Insert the metrics that are about the arm's own key exchange.
fn kex_metrics(bmf: &mut Bmf, bench: &BenchRun) {
    if let Some(stats) = Stats::from_micros(&bench.kex_us) {
        insert(
            bmf,
            format!("kex/{}", bench.kex_algorithm),
            Measure::LatencyUs,
            Metric::range(stats.median, stats.min, stats.max),
        );
    }
}

/// Insert the metrics that are about the device rather than the key exchange,
/// from the one arm that speaks for it.
fn device_metrics(bmf: &mut Bmf, bench: &BenchRun) {
    if let Some(stats) = Stats::from_micros(&bench.rtt_us) {
        insert(
            bmf,
            "bridge/rtt",
            Measure::LatencyUs,
            Metric::range(stats.median, stats.min, stats.max),
        );
    }
    if let Some(ready) = bench.boot_t(cmd::TCP_LISTENING) {
        insert(bmf, "boot", Measure::LatencyUs, Metric::point(ready as f64));
    }
    if let (Some(peripherals), Some(wifi)) = (
        bench.boot_t(cmd::PERIPHERALS_READY),
        bench.boot_t(cmd::WIFI_UP),
    ) && wifi >= peripherals
    {
        insert(
            bmf,
            "boot/wifi-association",
            Measure::LatencyUs,
            Metric::point((wifi - peripherals) as f64),
        );
    }
    for heap in &bench.heap {
        insert(
            bmf,
            format!("heap/{}", heap.label),
            Measure::HeapBytes,
            Metric::point(heap.used_bytes as f64),
        );
    }
}

/// Insert the size metrics from the results.
fn size_metrics(bmf: &mut Bmf, size: &SocSize) {
    insert(
        bmf,
        format!("{}/size/flash", size.profile),
        Measure::FlashBytes,
        Metric::point(size.flash_bytes as f64),
    );
    insert(
        bmf,
        format!("{}/size/ram", size.profile),
        Measure::RamBytes,
        Metric::point(size.ram_bytes as f64),
    );
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unreadable_literal)]
mod tests {
    use super::*;
    use crate::cmd;
    use crate::results::{
        BenchResults, BootCheckpoint, HeapSnapshot, RunOutcome, SizeResults,
    };

    fn metric<'a>(bmf: &'a Bmf, bench: &str, measure: Measure) -> &'a Metric {
        bmf.get(bench).unwrap().get(&measure).unwrap()
    }

    fn results(run: BenchRun) -> Results {
        Results::Bench(BenchResults {
            features: "f".into(),
            runs: vec![RunOutcome::Ok {
                heap_size: None,
                run,
            }],
        })
    }

    fn bench(kex: Vec<u64>) -> BenchRun {
        BenchRun {
            kex_algorithm: "mlkem768x25519-sha256".into(),
            boot: vec![
                BootCheckpoint {
                    label: cmd::PERIPHERALS_READY.into(),
                    t_abs_us: 50000,
                },
                BootCheckpoint {
                    label: cmd::WIFI_UP.into(),
                    t_abs_us: 300000,
                },
                BootCheckpoint {
                    label: cmd::TCP_LISTENING.into(),
                    t_abs_us: 412300,
                },
            ],
            heap: vec![HeapSnapshot {
                label: "wifi_up".into(),
                used_bytes: 53900,
                total_bytes: 73728,
                max_bytes: 54000,
            }],
            stack: vec![],
            kex_us: kex,
            rtt_us: vec![],
        }
    }

    fn size(soc: &str, flash: u64) -> SocSize {
        SocSize {
            soc: soc.into(),
            board: soc.into(),
            profile: "release".into(),
            target: "target".into(),
            features: soc.into(),
            flash_bytes: flash,
            ram_bytes: 180224,
            stack_reserved_bytes: 228864,
            crates: vec![],
        }
    }

    #[test]
    fn kex_bmf() {
        let bmf = to_bmf(&[results(bench(vec![100, 200, 300, 400]))]);
        let metric_result = metric(&bmf, "kex/mlkem768x25519-sha256", Measure::LatencyUs);
        assert_eq!(metric_result.value, 250.0);
        assert_eq!(
            (metric_result.lower_value, metric_result.upper_value),
            (Some(100.0), Some(400.0))
        );

        let classical = BenchRun {
            kex_algorithm: cmd::REFERENCE_KEX.into(),
            ..bench(vec![240000])
        };
        let bmf = to_bmf(&[results(bench(vec![285000])), results(classical)]);
        assert_eq!(
            metric(&bmf, "kex/mlkem768x25519-sha256", Measure::LatencyUs).value,
            285000.0
        );
        assert_eq!(
            metric(&bmf, "kex/curve25519-sha256", Measure::LatencyUs).value,
            240000.0
        );
    }

    #[test]
    fn bench_has_boot() {
        let bmf = to_bmf(&[results(bench(vec![1]))]);
        assert_eq!(metric(&bmf, "boot", Measure::LatencyUs).value, 412300.0);
        assert_eq!(
            metric(&bmf, "boot/wifi-association", Measure::LatencyUs).value,
            250000.0
        );
        assert_eq!(
            metric(&bmf, "heap/wifi_up", Measure::HeapBytes).value,
            53900.0
        );
    }

    #[test]
    fn no_kex_samples() {
        let bmf = to_bmf(&[results(bench(vec![]))]);
        assert!(!bmf.contains_key("kex/mlkem768x25519-sha256"));
        assert!(bmf.contains_key("boot"));
    }

    #[test]
    fn rtt_samples() {
        let with_rtt = BenchRun {
            rtt_us: vec![10000, 12000, 20000],
            ..bench(vec![])
        };
        let bmf = to_bmf(&[results(with_rtt)]);
        let m = metric(&bmf, "bridge/rtt", Measure::LatencyUs);
        assert_eq!(m.value, 12000.0);
        assert_eq!(
            (m.lower_value, m.upper_value),
            (Some(10000.0), Some(20000.0))
        );
    }

    #[test]
    fn size_names() {
        let bmf = to_bmf(&[Results::Size(SizeResults {
            entries: vec![size("esp32c6", 1046528)],
        })]);
        assert_eq!(
            metric(&bmf, "release/size/flash", Measure::FlashBytes).value,
            1046528.0
        );
        assert_eq!(
            metric(&bmf, "release/size/ram", Measure::RamBytes).value,
            180224.0
        );
        assert_eq!(
            metric(&bmf, "release/size/flash", Measure::FlashBytes).lower_value,
            None
        );
    }

    #[test]
    fn two_profiles_one_soc() {
        let min = SocSize {
            profile: "release-min".into(),
            flash_bytes: 60,
            ..size("esp32c6", 0)
        };
        let bmf = to_bmf(&[Results::Size(SizeResults {
            entries: vec![size("esp32c6", 100), min],
        })]);
        assert_eq!(
            metric(&bmf, "release/size/flash", Measure::FlashBytes).value,
            100.0
        );
        assert_eq!(
            metric(&bmf, "release-min/size/flash", Measure::FlashBytes).value,
            60.0
        );
    }

    #[test]
    fn stack_takes_the_maximum() {
        let with_stack = BenchRun {
            stack: vec![
                StackSnapshot {
                    label: "boot".into(),
                    max_bytes: 9000,
                    reserved_bytes: 247408,
                },
                StackSnapshot {
                    label: "session".into(),
                    max_bytes: 38200,
                    reserved_bytes: 247408,
                },
            ],
            ..bench(vec![])
        };
        let bmf = to_bmf(&[results(with_stack)]);
        assert_eq!(
            metric(&bmf, "stack/max", Measure::StackBytes).value,
            38200.0
        );

        let stack = |max_bytes| StackSnapshot {
            label: "session".into(),
            max_bytes,
            reserved_bytes: 247408,
        };
        let reference = BenchRun {
            kex_algorithm: cmd::REFERENCE_KEX.into(),
            stack: vec![stack(30000)],
            ..bench(vec![240000])
        };
        let mlkem = BenchRun {
            stack: vec![stack(38200)],
            ..bench(vec![285000])
        };
        let bmf = to_bmf(&[results(reference), results(mlkem)]);
        assert_eq!(
            metric(&bmf, "stack/max", Measure::StackBytes).value,
            38200.0
        );
    }

    #[test]
    fn reference_is_device() {
        let reference = BenchRun {
            kex_algorithm: cmd::REFERENCE_KEX.into(),
            rtt_us: vec![10000],
            ..bench(vec![240000])
        };
        let mlkem = BenchRun {
            rtt_us: vec![30000],
            ..bench(vec![285000])
        };
        let bmf = to_bmf(&[results(reference), results(mlkem)]);
        assert_eq!(
            metric(&bmf, "bridge/rtt", Measure::LatencyUs).value,
            10000.0
        );
    }

    #[test]
    fn merge_runs() {
        let classical = || BenchRun {
            kex_algorithm: cmd::REFERENCE_KEX.into(),
            rtt_us: vec![10000],
            stack: vec![StackSnapshot {
                label: "session".into(),
                max_bytes: 30000,
                reserved_bytes: 247408,
            }],
            ..bench(vec![240000])
        };
        let mlkem = || BenchRun {
            rtt_us: vec![30000],
            stack: vec![StackSnapshot {
                label: "session".into(),
                max_bytes: 38200,
                reserved_bytes: 247408,
            }],
            ..bench(vec![285000])
        };
        assert_eq!(
            to_bmf(&[results(classical()), results(mlkem())]),
            to_bmf(&[results(mlkem()), results(classical())])
        );
    }

    #[test]
    fn heap_no_metrics() {
        let probe = Results::Bench(BenchResults {
            features: "f".into(),
            runs: vec![
                RunOutcome::Failed {
                    heap_size: Some(49_152),
                    ready: false,
                },
                RunOutcome::Ok {
                    heap_size: Some(65_536),
                    run: bench(vec![285700]),
                },
            ],
        });
        assert!(to_bmf(&[probe]).is_empty());
    }

    #[test]
    fn multiple_inputs() {
        let bmf = to_bmf(&[
            results(bench(vec![285700])),
            Results::Size(SizeResults {
                entries: vec![size("esp32c6", 1046528)],
            }),
        ]);
        assert!(bmf.contains_key("kex/mlkem768x25519-sha256"));
        assert!(bmf.contains_key("release/size/flash"));
    }

    #[test]
    fn point_metrics() {
        assert_eq!(
            serde_json::to_value(Metric::point(1.0)).unwrap(),
            serde_json::json!({ "value": 1.0 })
        );
        assert_eq!(
            serde_json::to_value(Metric::range(2.0, 1.0, 3.0)).unwrap(),
            serde_json::json!({ "value": 2.0, "lower_value": 1.0, "upper_value": 3.0 })
        );
    }
}
