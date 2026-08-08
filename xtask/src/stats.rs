// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Sample statistics and human formatting for the report.
//!
//! Deliberately minimal: median plus the observed `[min, max]` range and `n`.
//! An earlier version carried a trimmed mean and a hand-rolled
//! signal-vs-noise "verdict" classifier, both ported from the retired
//! `scripts/bench-*.sh` `awk` blocks. Neither is needed now — bit-compatibility
//! with those scripts stopped being a constraint when they were deleted, and
//! deciding whether a change is a real regression is Bencher's job (it has the
//! run history and configurable statistical thresholds; a single run does not).

/// Summary statistics over a set of samples.
#[derive(Debug, Clone, Copy)]
pub struct Stats {
    pub n: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    /// Nearest-rank 95th percentile — the tail figure for latency samples,
    /// where a max distorted by one straggler says little.
    pub p95: f64,
    pub stddev: f64,
}

impl Stats {
    /// Computes statistics over `samples`. Returns `None` for an empty slice.
    pub fn from_samples(samples: &[f64]) -> Option<Stats> {
        let n = samples.len();
        if n == 0 {
            return None;
        }
        let mut v = samples.to_vec();
        v.sort_by(|a, b| a.partial_cmp(b).expect("samples must be finite"));

        let mean = v.iter().sum::<f64>() / n as f64;
        let median = if n % 2 == 1 {
            v[n / 2]
        } else {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        };
        // Sample standard deviation (n-1 denominator), 0 for a single sample.
        let stddev = if n > 1 {
            let ss: f64 = v.iter().map(|x| (x - mean) * (x - mean)).sum();
            (ss / (n as f64 - 1.0)).sqrt()
        } else {
            0.0
        };

        // Nearest-rank: the smallest sample ≥ 95% of the others.
        let p95 = v[(0.95 * n as f64).ceil() as usize - 1];

        Some(Stats {
            n,
            min: v[0],
            max: v[n - 1],
            mean,
            median,
            p95,
            stddev,
        })
    }

    /// Convenience for the many `Vec<u64>` sample sets coming off the device.
    pub fn from_micros(samples: &[u64]) -> Option<Stats> {
        let f: Vec<f64> = samples.iter().map(|&s| s as f64).collect();
        Stats::from_samples(&f)
    }
}

/// Formats a microsecond value: `<1 ms` as integer µs, `<1 s` as `.1f ms`,
/// otherwise `.2f s`.
pub fn fmt_us(v: f64) -> String {
    if v < 1000.0 {
        format!("{v:.0} µs")
    } else if v < 1_000_000.0 {
        format!("{:.1} ms", v / 1000.0)
    } else {
        format!("{:.2} s", v / 1_000_000.0)
    }
}

/// Like [`fmt_us`] but keeps an explicit sign, for deltas.
pub fn fmt_signed_us(v: f64) -> String {
    if v < 0.0 {
        format!("−{}", fmt_us(-v))
    } else {
        format!("+{}", fmt_us(v))
    }
}

/// Formats a byte count: `<1 KiB` as integer bytes, `<1 MiB` as `.1f KiB`,
/// otherwise `.2f MiB`. Negative values keep their sign.
pub fn fmt_b(v: f64) -> String {
    let (sign, v) = if v < 0.0 { ("-", -v) } else { ("", v) };
    if v < 1024.0 {
        format!("{sign}{v:.0} B")
    } else if v < 1_048_576.0 {
        format!("{sign}{:.1} KiB", v / 1024.0)
    } else {
        format!("{sign}{:.2} MiB", v / 1_048_576.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_handles_odd_and_even_n() {
        assert_eq!(
            Stats::from_samples(&[10.0, 30.0, 20.0]).unwrap().median,
            20.0
        );
        assert_eq!(
            Stats::from_samples(&[10.0, 40.0, 20.0, 30.0])
                .unwrap()
                .median,
            25.0
        );
    }

    #[test]
    fn reports_range_and_spread() {
        let s = Stats::from_samples(&[100.0, 1.0, 3.0, 2.0]).unwrap();
        assert_eq!((s.n, s.min, s.max), (4, 1.0, 100.0));
        assert_eq!(s.mean, 26.5);
        // Median resists the outlier the mean does not.
        assert_eq!(s.median, 2.5);
        assert!(s.stddev > 0.0);
    }

    #[test]
    fn single_sample_has_no_spread() {
        let s = Stats::from_samples(&[5.0]).unwrap();
        assert_eq!(s.stddev, 0.0);
        assert_eq!(s.min, s.max);
        assert_eq!(s.p95, 5.0);
        assert!(Stats::from_samples(&[]).is_none());
    }

    #[test]
    fn p95_is_nearest_rank() {
        // 20 samples: rank ⌈0.95·20⌉ = 19 → the 19th smallest, not the max.
        let v: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(Stats::from_samples(&v).unwrap().p95, 19.0);
    }

    #[test]
    fn formats_across_unit_boundaries() {
        assert_eq!(fmt_us(999.0), "999 µs");
        assert_eq!(fmt_us(1500.0), "1.5 ms");
        assert_eq!(fmt_us(2_500_000.0), "2.50 s");
        assert_eq!(fmt_b(512.0), "512 B");
        assert_eq!(fmt_b(2048.0), "2.0 KiB");
        assert_eq!(fmt_signed_us(-1500.0), "−1.5 ms");
    }
}
