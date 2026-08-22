// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The statistics calculated from the benchmarking results.

use humansize::{BINARY, format_size};
use statrs::statistics::{Data, Max, Median, Min};

/// Summary statistics over a set of samples.
#[derive(Debug, Clone, Copy)]
pub struct Stats {
    /// The number of samples.
    pub n: usize,
    /// Minimum of the batch.
    pub min: f64,
    /// Maximum of the batch.
    pub max: f64,
    /// Median of the batch.
    pub median: f64,
}

impl Stats {
    /// Computes stats over the `samples` from f64 values. Returns `None` for an empty slice.
    pub fn from_samples(samples: &[f64]) -> Option<Stats> {
        if samples.is_empty() {
            return None;
        }
        let data = Data::new(samples.to_vec());

        Some(Stats {
            n: samples.len(),
            min: data.min(),
            max: data.max(),
            median: data.median(),
        })
    }

    /// Computes stats over the `samples` from u64 values. Returns `None` for an empty slice.
    pub fn from_micros(samples: &[u64]) -> Option<Stats> {
        let f: Vec<f64> = samples.iter().copied().map(to_f64).collect();
        Stats::from_samples(&f)
    }
}

/// Converts a measured value to f64.
pub fn to_f64(value: u64) -> f64 {
    f64::from(u32::try_from(value).expect("measured value to fit in u32"))
}

/// Formats a microsecond value.
pub fn fmt_us(v: f64) -> String {
    if v.round() < 1000.0 {
        format!("{v:.0} µs")
    } else if (v / 100.0).round() < 10_000.0 {
        format!("{:.1} ms", v / 1000.0)
    } else {
        format!("{:.2} s", v / 1_000_000.0)
    }
}

/// Formats a byte count.
pub fn fmt_bytes(v: u64) -> String {
    format_size(v, BINARY)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn compute_stats() {
        let stats = Stats::from_samples(&[100.0, 1.0, 3.0, 2.0]).unwrap();
        assert_eq!(
            (stats.n, stats.min, stats.max, stats.median),
            (4, 1.0, 100.0, 2.5)
        );

        let stats = Stats::from_samples(&[5.0]).unwrap();
        assert_eq!(stats.min, stats.max);

        assert!(Stats::from_samples(&[]).is_none());
    }
}
