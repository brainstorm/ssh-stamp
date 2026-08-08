// SPDX-FileCopyrightText: 2026 Marko Malenic mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask bmf` — convert collected `results.json` into Bencher Metric Format.
//!
//! Milestone (d): [Bencher](https://bencher.dev) is the single system-of-record
//! for the project's tracked metrics — boot time, KEX wall time, heap high-water,
//! static stack frames, flash/RAM size, and sweep throughput. Its `json` adapter
//! ingests an arbitrary benchmark → measure → value map (BMF), so this
//! subcommand is the emitter: xtask produces the JSON, `bencher run --adapter
//! json --file` uploads it, and configured thresholds fail the PR on a
//! regression.
//!
//! The host crypto benches feed the *same* Bencher project through its
//! `rust_criterion` adapter, so there is one dashboard and one alerting path
//! rather than a second service for the host half.
//!
//! BMF shape (one object, sorted for stable diffs):
//!
//! ```json
//! {
//!   "kex/mlkem768x25519-sha256": { "latency-us": { "value": 285700.0, "lower_value": …, "upper_value": … } },
//!   "boot/default":              { "latency-us": { "value": 412300.0 } },
//!   "size/flash":                { "flash-bytes": { "value": 1046528.0 } }
//! }
//! ```
//!
//! The per-board dimension is Bencher's *Testbed*, supplied on the `bencher run`
//! command line — not encoded here — so benchmark names stay board-agnostic. The
//! one exception is `size`/`stack`, which are inherently per-SoC and per-profile:
//! when a single invocation mixes either (a `size --all` dump, or the shipping
//! and `release-min` profiles together) their names gain the segment that tells
//! them apart, so neither can silently overwrite the other.

use crate::results::{BenchResults, Results, SocSize, SocStack, SweepResults};
use crate::stats::Stats;
use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Bencher measure slugs. Direction (smaller/larger is better) and units are
/// configured on the measure in the Bencher project; the BMF carries only the
/// raw value, so these names are the join key. Bencher auto-creates a measure
/// the first time it sees a new slug.
const M_LATENCY_US: &str = "latency-us";
const M_HEAP_B: &str = "heap-bytes";
const M_STACK_B: &str = "stack-bytes";
const M_FLASH_B: &str = "flash-bytes";
const M_RAM_B: &str = "ram-bytes";
const M_THROUGHPUT: &str = "throughput-kib-s";

#[derive(ClapArgs)]
pub struct Args {
    /// Results JSON file(s) to convert (repeatable). Produced by any of
    /// `xtask bench` / `size` / `stack` / `sweep --json`.
    #[arg(long = "input", required = true)]
    inputs: Vec<PathBuf>,
    /// Write the BMF JSON here (defaults to stdout).
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<()> {
    let results = args
        .inputs
        .iter()
        .map(|p| Results::read(p))
        .collect::<Result<Vec<_>>>()?;

    let bmf = to_bmf(&results);
    if bmf.is_empty() {
        bail!("no metrics extracted from the provided --input files");
    }

    let json = serde_json::to_string_pretty(&bmf)?;
    match &args.output {
        Some(out) => {
            std::fs::write(out, &json).with_context(|| format!("writing {}", out.display()))?;
            eprintln!("wrote {} ({} benchmarks)", out.display(), bmf.len());
        }
        None => println!("{json}"),
    }
    Ok(())
}

/// One BMF metric: a point value, optionally with a lower/upper bound (the
/// spread Bencher draws error bars from and can threshold on).
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Metric {
    value: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    lower_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upper_value: Option<f64>,
}

impl Metric {
    fn point(value: f64) -> Self {
        Self {
            value,
            lower_value: None,
            upper_value: None,
        }
    }

    fn range(value: f64, lower: f64, upper: f64) -> Self {
        Self {
            value,
            lower_value: Some(lower),
            upper_value: Some(upper),
        }
    }
}

/// A BMF document: benchmark name → measure slug → metric. `BTreeMap`s keep the
/// output deterministic (stable diffs, stable test assertions).
type Bmf = BTreeMap<String, BTreeMap<String, Metric>>;

/// Records `metric` under `bench`/`measure`, creating the benchmark on first use.
fn insert(bmf: &mut Bmf, bench: impl Into<String>, measure: &str, metric: Metric) {
    bmf.entry(bench.into())
        .or_default()
        .insert(measure.to_string(), metric);
}

/// Converts every result record into the merged BMF document.
fn to_bmf(results: &[Results]) -> Bmf {
    // `size`/`stack` are per-SoC *and* per-profile; only disambiguate their names
    // when a single document actually mixes one or the other (otherwise the
    // testbed dimension already carries the board, the profile is implicit, and
    // the plain name is cleaner). Without the profile segment a shipping and a
    // `release-min` measurement of the same SoC collide on `size/flash` and the
    // second silently overwrites the first.
    let mixes = |f: fn(&Results) -> Vec<&str>| -> bool {
        results.iter().flat_map(f).collect::<BTreeSet<_>>().len() > 1
    };
    let size_socs = mixes(|r| match r {
        Results::Size(s) => s.entries.iter().map(|e| e.soc.as_str()).collect(),
        _ => Vec::new(),
    });
    let size_profiles = mixes(|r| match r {
        Results::Size(s) => s.entries.iter().map(|e| e.profile.as_str()).collect(),
        _ => Vec::new(),
    });
    let stack_socs = mixes(|r| match r {
        Results::Stack(s) => s.entries.iter().map(|e| e.soc.as_str()).collect(),
        _ => Vec::new(),
    });
    let stack_profiles = mixes(|r| match r {
        Results::Stack(s) => s.entries.iter().map(|e| e.profile.as_str()).collect(),
        _ => Vec::new(),
    });

    let mut bmf = Bmf::new();
    for r in results {
        match r {
            Results::Bench(b) => bench_metrics(&mut bmf, b),
            Results::Size(s) => {
                for e in &s.entries {
                    size_metrics(
                        &mut bmf,
                        e,
                        prefix(&e.soc, size_socs, &e.profile, size_profiles),
                    );
                }
            }
            Results::Stack(s) => {
                for e in &s.entries {
                    stack_metrics(
                        &mut bmf,
                        e,
                        prefix(&e.soc, stack_socs, &e.profile, stack_profiles),
                    );
                }
            }
            Results::Sweep(s) => sweep_metrics(&mut bmf, s),
        }
    }
    bmf
}

/// `kex/<negotiated-algorithm>` → `latency-us` (median, with min/max as the
/// range) — keyed by what the sessions actually measured, not by the build
/// variant, so the two arms of a `bench --kex` A/B chart as separate series;
/// two result sets that negotiated the same algorithm are the same measurement
/// and merge. `bridge/rtt` → the loopback round-trip latency (median, min/max).
/// The build-shaped metrics stay variant-keyed: `boot/<variant>` → cold-boot
/// latency plus a `wifi-assoc` sub-benchmark, and one `heap/<variant>/<label>`
/// → `heap-bytes` per startup snapshot.
fn bench_metrics(bmf: &mut Bmf, b: &BenchResults) {
    if let Some(s) = Stats::from_micros(&b.kex_us) {
        insert(
            bmf,
            format!("kex/{}", b.kex_algorithm.as_deref().unwrap_or("unknown")),
            M_LATENCY_US,
            Metric::range(s.median, s.min, s.max),
        );
    }
    if let Some(s) = Stats::from_micros(&b.rtt_us) {
        insert(
            bmf,
            "bridge/rtt",
            M_LATENCY_US,
            Metric::range(s.median, s.min, s.max),
        );
    }
    if let Some(ready) = b.boot_t("bench_tcp_listening") {
        insert(
            bmf,
            format!("boot/{}", b.variant),
            M_LATENCY_US,
            Metric::point(ready as f64),
        );
    }
    if let (Some(peripherals), Some(wifi)) = (
        b.boot_t("bench_peripherals_ready"),
        b.boot_t("bench_wifi_up"),
    ) && wifi >= peripherals
    {
        insert(
            bmf,
            format!("boot/{}/wifi-assoc", b.variant),
            M_LATENCY_US,
            Metric::point((wifi - peripherals) as f64),
        );
    }
    for h in &b.heap {
        insert(
            bmf,
            format!("heap/{}/{}", b.variant, h.label),
            M_HEAP_B,
            Metric::point(h.used_bytes as f64),
        );
    }
}

/// `size/flash` + `size/ram` → `flash-bytes` / `ram-bytes`.
fn size_metrics(bmf: &mut Bmf, e: &SocSize, prefix: String) {
    insert(
        bmf,
        format!("{prefix}size/flash"),
        M_FLASH_B,
        Metric::point(e.flash_b as f64),
    );
    insert(
        bmf,
        format!("{prefix}size/ram"),
        M_RAM_B,
        Metric::point(e.ram_b as f64),
    );
}

/// `stack/max-frame` → `stack-bytes` (largest single static frame).
fn stack_metrics(bmf: &mut Bmf, e: &SocStack, prefix: String) {
    insert(
        bmf,
        format!("{prefix}stack/max-frame"),
        M_STACK_B,
        Metric::point(e.max_frame_b as f64),
    );
}

/// `sweep/<knob>` → the recommended value's throughput and measured static RAM.
fn sweep_metrics(bmf: &mut Bmf, s: &SweepResults) {
    let Some(recommended) = s.recommended else {
        return;
    };
    let Some(point) = s.points.iter().find(|p| p.value == recommended) else {
        return;
    };
    let name = format!("sweep/{}", s.knob);
    insert(
        bmf,
        name.clone(),
        M_THROUGHPUT,
        Metric::point(point.throughput_kib_s),
    );
    insert(bmf, name, M_RAM_B, Metric::point(point.ram_b as f64));
}

/// `"<soc>/"` and/or `"<profile>/"`, each included only when the document
/// actually mixes them — otherwise the testbed carries the board and the single
/// profile is implicit.
fn prefix(soc: &str, mixes_socs: bool, profile: &str, mixes_profiles: bool) -> String {
    [(soc, mixes_socs), (profile, mixes_profiles)]
        .iter()
        .filter(|(_, mixes)| *mixes)
        .map(|(seg, _)| format!("{seg}/"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::results::{
        BootCheckpoint, HeapSnapshot, SizeResults, StackResults, SweepLoad, SweepPoint,
    };

    fn metric<'a>(bmf: &'a Bmf, bench: &str, measure: &str) -> &'a Metric {
        bmf.get(bench)
            .unwrap_or_else(|| panic!("missing benchmark {bench}"))
            .get(measure)
            .unwrap_or_else(|| panic!("missing measure {measure} on {bench}"))
    }

    fn bench(kex: Vec<u64>) -> BenchResults {
        BenchResults {
            variant: "default".into(),
            kex_algorithm: Some("mlkem768x25519-sha256".into()),
            host: "h".into(),
            user: "u".into(),
            sessions_driven: 4,
            failures: 0,
            boot: vec![
                BootCheckpoint {
                    name: "bench_peripherals_ready".into(),
                    t_abs_us: 50_000,
                },
                BootCheckpoint {
                    name: "bench_wifi_up".into(),
                    t_abs_us: 300_000,
                },
                BootCheckpoint {
                    name: "bench_tcp_listening".into(),
                    t_abs_us: 412_300,
                },
            ],
            sessions: vec![],
            heap: vec![HeapSnapshot {
                label: "wifi_up".into(),
                used_bytes: 53_900,
                total_bytes: 73_728,
                max_bytes: 54_000,
            }],
            kex_us: kex,
            rtt_us: vec![],
        }
    }

    fn size(soc: &str, flash: u64) -> SocSize {
        SocSize {
            soc: soc.into(),
            profile: "release".into(),
            target: "t".into(),
            features: soc.into(),
            flash_b: flash,
            ram_b: 180_224,
            sections: vec![],
            crates: vec![],
        }
    }

    #[test]
    fn kex_uses_median_with_min_max_range() {
        let bmf = to_bmf(&[Results::Bench(bench(vec![100, 200, 300, 400]))]);
        let m = metric(&bmf, "kex/mlkem768x25519-sha256", M_LATENCY_US);
        assert_eq!(m.value, 250.0);
        assert_eq!((m.lower_value, m.upper_value), (Some(100.0), Some(400.0)));
    }

    #[test]
    fn kex_keys_on_the_negotiated_algorithm_not_the_variant() {
        // The two arms of a `bench --kex` A/B come from the same build variant;
        // only the negotiated algorithm tells them apart.
        let classical = BenchResults {
            kex_algorithm: Some("curve25519-sha256".into()),
            ..bench(vec![240_000])
        };
        let bmf = to_bmf(&[
            Results::Bench(bench(vec![285_000])),
            Results::Bench(classical),
        ]);
        assert_eq!(
            metric(&bmf, "kex/mlkem768x25519-sha256", M_LATENCY_US).value,
            285_000.0
        );
        assert_eq!(
            metric(&bmf, "kex/curve25519-sha256", M_LATENCY_US).value,
            240_000.0
        );
        assert!(!bmf.contains_key("kex/default"));
    }

    #[test]
    fn bench_maps_boot_wifi_and_heap() {
        let bmf = to_bmf(&[Results::Bench(bench(vec![1]))]);
        assert_eq!(metric(&bmf, "boot/default", M_LATENCY_US).value, 412_300.0);
        // wifi-assoc = wifi_up - peripherals_ready.
        assert_eq!(
            metric(&bmf, "boot/default/wifi-assoc", M_LATENCY_US).value,
            250_000.0
        );
        assert_eq!(
            metric(&bmf, "heap/default/wifi_up", M_HEAP_B).value,
            53_900.0
        );
    }

    #[test]
    fn no_kex_samples_emit_no_kex_metric() {
        let bmf = to_bmf(&[Results::Bench(bench(vec![]))]);
        assert!(!bmf.contains_key("kex/mlkem768x25519-sha256"));
        // The boot metrics are still there.
        assert!(bmf.contains_key("boot/default"));
    }

    #[test]
    fn rtt_samples_map_to_bridge_rtt() {
        let with_rtt = BenchResults {
            rtt_us: vec![10_000, 12_000, 20_000],
            ..bench(vec![])
        };
        let bmf = to_bmf(&[Results::Bench(with_rtt)]);
        let m = metric(&bmf, "bridge/rtt", M_LATENCY_US);
        assert_eq!(m.value, 12_000.0);
        assert_eq!(
            (m.lower_value, m.upper_value),
            (Some(10_000.0), Some(20_000.0))
        );
    }

    #[test]
    fn single_soc_size_names_are_unprefixed() {
        let bmf = to_bmf(&[Results::Size(SizeResults {
            entries: vec![size("esp32c6", 1_046_528)],
            overflow_checks_enforced_by: vec!["cfg(all())".into()],
        })]);
        assert_eq!(metric(&bmf, "size/flash", M_FLASH_B).value, 1_046_528.0);
        assert_eq!(metric(&bmf, "size/ram", M_RAM_B).value, 180_224.0);
        assert_eq!(metric(&bmf, "size/flash", M_FLASH_B).lower_value, None);
    }

    #[test]
    fn multi_soc_size_names_are_soc_prefixed() {
        let bmf = to_bmf(&[Results::Size(SizeResults {
            entries: vec![size("esp32c6", 100), size("esp32c3", 200)],
            overflow_checks_enforced_by: vec![],
        })]);
        assert_eq!(metric(&bmf, "esp32c6/size/flash", M_FLASH_B).value, 100.0);
        assert_eq!(metric(&bmf, "esp32c3/size/flash", M_FLASH_B).value, 200.0);
        assert!(!bmf.contains_key("size/flash"));
    }

    #[test]
    fn one_soc_two_profiles_are_profile_prefixed() {
        // The shipping and size-minimised measurements of the same SoC are
        // routinely converted together; without the profile segment the second
        // would silently overwrite the first.
        let min = SocSize {
            profile: "release-min".into(),
            flash_b: 60,
            ..size("esp32c6", 0)
        };
        let bmf = to_bmf(&[Results::Size(SizeResults {
            entries: vec![size("esp32c6", 100), min],
            overflow_checks_enforced_by: vec![],
        })]);
        assert_eq!(metric(&bmf, "release/size/flash", M_FLASH_B).value, 100.0);
        assert_eq!(
            metric(&bmf, "release-min/size/flash", M_FLASH_B).value,
            60.0
        );
        assert!(!bmf.contains_key("size/flash"));
    }

    #[test]
    fn stack_maps_to_max_frame() {
        let bmf = to_bmf(&[Results::Stack(StackResults {
            entries: vec![SocStack {
                soc: "esp32c6".into(),
                profile: "stack-analysis".into(),
                target: "t".into(),
                functions: vec![],
                total_functions: 0,
                max_frame_b: 4_096,
            }],
        })]);
        assert_eq!(metric(&bmf, "stack/max-frame", M_STACK_B).value, 4_096.0);
    }

    fn sweep(recommended: Option<u64>) -> SweepResults {
        SweepResults {
            host: "h".into(),
            user: "u".into(),
            features: "f".into(),
            load: SweepLoad {
                concurrency: 4,
                payload_kib: 256,
                duration_s: 20,
            },
            knob: "heap_size".into(),
            bisect: false,
            tolerance: 0.9,
            recommended,
            points: vec![SweepPoint {
                value: 65_536,
                ram_b: 77_136,
                heap_max_b: Some(60_000),
                ready: true,
                oom: false,
                throughput_kib_s: 100.0,
                bytes_sent: 1,
                failures: 0,
            }],
        }
    }

    #[test]
    fn sweep_reports_the_recommended_points_own_numbers() {
        let bmf = to_bmf(&[Results::Sweep(sweep(Some(65_536)))]);
        assert_eq!(metric(&bmf, "sweep/heap_size", M_THROUGHPUT).value, 100.0);
        // Measured static RAM of that image, not an estimate.
        assert_eq!(metric(&bmf, "sweep/heap_size", M_RAM_B).value, 77_136.0);
    }

    #[test]
    fn sweep_without_recommendation_emits_nothing() {
        assert!(to_bmf(&[Results::Sweep(sweep(None))]).is_empty());
    }

    #[test]
    fn multiple_inputs_merge_into_one_document() {
        let bmf = to_bmf(&[
            Results::Bench(bench(vec![285_700])),
            Results::Size(SizeResults {
                entries: vec![size("esp32c6", 1_046_528)],
                overflow_checks_enforced_by: vec![],
            }),
        ]);
        assert!(bmf.contains_key("kex/mlkem768x25519-sha256"));
        assert!(bmf.contains_key("size/flash"));
    }

    #[test]
    fn point_metric_serializes_without_bounds() {
        assert_eq!(
            serde_json::to_string(&Metric::point(1.0)).unwrap(),
            r#"{"value":1.0}"#
        );
        assert_eq!(
            serde_json::to_string(&Metric::range(2.0, 1.0, 3.0)).unwrap(),
            r#"{"value":2.0,"lower_value":1.0,"upper_value":3.0}"#
        );
    }
}
