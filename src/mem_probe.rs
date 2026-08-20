// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The benchmarking instrumentation, including checkpoints and replay. Timing is
//! not related to the platform in any way, so it doesn't need to live inside the
//! binrary crate.
//!

#[cfg(feature = "mem-probe")]
use embassy_time::Instant;
#[cfg(feature = "mem-probe")]
use portable_atomic::{AtomicU64, Ordering};

/// Emits a structured benchmark line.
#[macro_export]
macro_rules! bench_emit {
    ($($arg:tt)*) => {
        ::log::info!("@BENCH {}", ::core::format_args!($($arg)*))
    };
}

/// One point in the `ssh_stamp` flow.
#[derive(Clone, Copy)]
pub enum Checkpoint {
    /// Boot reached.
    Boot,
    /// Core peripherals ready.
    PeripheralsReady,
    /// Wifi is up.
    WifiUp,
    /// TCP listener is ready.
    TcpListening,
    /// A TCP connection was accepted.
    TcpAccept,
    /// SSH key exchange finished.
    KexComplete,
    /// Client authenticated.
    AuthSuccess,
    /// A session channel was opened.
    ChannelOpen,
}

#[cfg(feature = "mem-probe")]
impl Checkpoint {
    /// The name emitted on the `@BENCH checkpoint=` line. The host uses these to
    /// compute the benchmark calculations.
    const fn name(self) -> &'static str {
        match self {
            Self::Boot => "bench_boot",
            Self::PeripheralsReady => "bench_peripherals_ready",
            Self::WifiUp => "bench_wifi_up",
            Self::TcpListening => "bench_tcp_listening",
            Self::TcpAccept => "bench_tcp_accept",
            Self::KexComplete => "bench_kex_complete",
            Self::AuthSuccess => "bench_auth_success",
            Self::ChannelOpen => "bench_channel_open",
        }
    }
}

/// Every [`Checkpoint`] in order, used for replaying checkpoints to the host.
#[cfg(feature = "mem-probe")]
const ALL: [Checkpoint; 8] = [
    Checkpoint::Boot,
    Checkpoint::PeripheralsReady,
    Checkpoint::WifiUp,
    Checkpoint::TcpListening,
    Checkpoint::TcpAccept,
    Checkpoint::KexComplete,
    Checkpoint::AuthSuccess,
    Checkpoint::ChannelOpen,
];

/// Timestamps indexed by [`Checkpoint`], used for replaying checkpoints to the host.
/// A timestamp of 0 indicates that nothing has been emitted yet.
#[cfg(feature = "mem-probe")]
static T_US: [AtomicU64; ALL.len()] = [const { AtomicU64::new(0) }; ALL.len()];

/// Logs the checkpoint.
#[cfg(feature = "mem-probe")]
pub fn checkpoint(c: Checkpoint) {
    let t_us = Instant::now().as_micros();
    bench_emit!("checkpoint={} t_us={t_us}", c.name());
    T_US[c as usize].store(t_us, Ordering::Relaxed);
}

#[cfg(not(feature = "mem-probe"))]
pub fn checkpoint(_c: Checkpoint) {}

/// Replays all checkpoints that have been recorded so far. This is needed because
/// the host may miss a logged line when resetting/flashing the device because the
/// console is not attached yet.
#[cfg(feature = "mem-probe")]
pub fn replay_checkpoints() {
    for c in ALL {
        // Zero is the "never fired" marker, not a timestamp.
        if let t_us @ 1.. = T_US[c as usize].load(Ordering::Relaxed) {
            crate::bench_emit!("checkpoint={} t_us={t_us}", c.name());
        }
    }
}

#[cfg(not(feature = "mem-probe"))]
pub fn replay_checkpoints() {}

#[cfg(feature = "mem-probe")]
static KEX_START_TICKS: AtomicU64 = AtomicU64::new(0);

/// Records the current instant as the KEX start point.
#[cfg(feature = "mem-probe")]
pub fn mark_kex_start() {
    KEX_START_TICKS.store(Instant::now().as_ticks(), Ordering::Relaxed);
}

#[cfg(not(feature = "mem-probe"))]
pub fn mark_kex_start() {}

/// Logs the elapsed time since `mark_kex_start`.
#[cfg(feature = "mem-probe")]
pub fn log_kex_elapsed(label: &str) {
    let start_ticks = KEX_START_TICKS.swap(0, Ordering::Relaxed);
    if start_ticks == 0 {
        return;
    }
    let elapsed = Instant::from_ticks(start_ticks).elapsed();
    bench_emit!("kex={label} elapsed_us={}", elapsed.as_micros());
}

#[cfg(not(feature = "mem-probe"))]
pub fn log_kex_elapsed(_label: &str) {}
