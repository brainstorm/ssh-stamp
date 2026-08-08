// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Subcommand implementations.

pub mod bench;
pub mod bmf;
pub mod crypto;
pub mod report;
pub mod size;
pub mod stack;
pub mod sweep;

/// The firmware build variants `xtask bench` can measure. Both carry
/// `mem-probe` so `@BENCH` lines are emitted; they differ only in `mlkem`.
///
/// A variant names what the image *is*; what a session *measured* (the
/// negotiated KEX algorithm) is recorded separately in the results, so the two
/// axes never conflate. `default` is the shipping feature set — `mlkem` named
/// explicitly, because `ssh-stamp-esp32` sets `default-features = false` on the
/// core crate precisely so that omitting it really does produce a no-mlkem
/// image. `nomlkem` is that omission, kept for the build-level A/B (a null
/// result until the sunset feature unification is fixed — see
/// docs/benchmarking.md). `crypto-bench` is deliberately excluded — its
/// boot-time microbench fires before the SSH loop, delaying the readiness
/// signal and adding noise to the first sample; `xtask crypto` measures it in
/// a dedicated run.
pub const VARIANTS: &[(&str, &str)] = &[
    ("default", "board-esp32c6-devkitc,mlkem,mem-probe"),
    ("nomlkem", "board-esp32c6-devkitc,mem-probe"),
];

/// Features for a named variant.
pub fn variant_features(variant: &str) -> Option<&'static str> {
    VARIANTS
        .iter()
        .find(|(v, _)| *v == variant)
        .map(|(_, f)| *f)
}

/// One row of the canonical boot→session checkpoint sequence.
pub struct CheckpointMeta {
    /// The `@BENCH checkpoint=<name>` value emitted by the firmware.
    pub name: &'static str,
    /// Human-friendly label for tables.
    pub label: &'static str,
    /// `true` for the one-shot startup checkpoints (`bench_boot` →
    /// `bench_tcp_listening`); `false` for the per-session ones.
    pub boot: bool,
    /// The `@BENCH heap=<label>` snapshot the firmware reports at this
    /// checkpoint, if any.
    pub heap_label: Option<&'static str>,
}

/// The eight checkpoints in firmware emission order. The first four fire once
/// at startup; the last four fire once per SSH session.
///
/// Keep in step with the `checkpoint!` invocations in `ssh_stamp::mem_probe` and
/// the `bench::log_heap` calls in the esp32 binary.
pub const CHECKPOINTS: &[CheckpointMeta] = &[
    CheckpointMeta {
        name: "bench_boot",
        label: "boot",
        boot: true,
        heap_label: Some("boot"),
    },
    CheckpointMeta {
        name: "bench_peripherals_ready",
        label: "peripherals ready",
        boot: true,
        heap_label: Some("peripherals"),
    },
    CheckpointMeta {
        name: "bench_wifi_up",
        label: "WiFi associated",
        boot: true,
        heap_label: Some("wifi_up"),
    },
    CheckpointMeta {
        name: "bench_tcp_listening",
        label: "TCP listening",
        boot: true,
        heap_label: None,
    },
    CheckpointMeta {
        name: "bench_tcp_accept",
        label: "TCP accept (SSH dialed in)",
        boot: false,
        heap_label: None,
    },
    CheckpointMeta {
        name: "bench_kex_complete",
        label: "KEX complete",
        boot: false,
        heap_label: None,
    },
    CheckpointMeta {
        name: "bench_auth_success",
        label: "pubkey auth success",
        boot: false,
        heap_label: None,
    },
    CheckpointMeta {
        name: "bench_channel_open",
        label: "session channel open",
        boot: false,
        heap_label: None,
    },
];

/// True if `name` is a one-shot startup checkpoint.
pub fn is_boot_checkpoint(name: &str) -> bool {
    CHECKPOINTS.iter().any(|c| c.name == name && c.boot)
}
