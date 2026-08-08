// SPDX-FileCopyrightText: 2025 Roman Valls, 2025
//
// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>

//! The crypto benchmarks, which benchmarks the effective code that would be running during an
//! SSH handshake. The benchmark measures the CPU cycle counter rather than a clock as it's more
//! precise for fast acting code.
//!

#[cfg(feature = "crypto-bench")]
use crate::bench_emit;
#[cfg(feature = "crypto-bench")]
use core::hint::black_box;
#[cfg(feature = "crypto-bench")]
use log::info;
#[cfg(feature = "crypto-bench")]
use ml_kem::{B32, DecapsulationKey, EncapsulationKey, Key, MlKem768, Seed, kem::KeyExport};

/// Runs the crypto benchmark table `iterations` times, emitting benchmark logs on the board.
///
/// `read_cycles` reads the CPU cycle counter and `cpu_mhz` is the clock frequency of the board to
/// convert the cycles to a time.
///
/// # Panics
///
/// This will panic if the encapsulation key fails to parse.
#[cfg(feature = "crypto-bench")]
pub fn run(iterations: u32, read_cycles: fn() -> u32, cpu_mhz: u32) {
    info!("crypto-bench: {iterations} iterations @ {cpu_mhz} MHz");

    // Setup code, seed is unused in the real server path.
    let seed: Seed = Seed::default();
    let dk: DecapsulationKey<MlKem768> = DecapsulationKey::from_seed(seed);
    let ek_bytes: Key<EncapsulationKey<MlKem768>> = dk.encapsulation_key().to_bytes();
    let m: B32 = B32::default();

    // Parse the client EK.
    measure("mlkem_ek_parse", iterations, read_cycles, cpu_mhz, || {
        let ek = EncapsulationKey::<MlKem768>::new(black_box(&ek_bytes))
            .expect("EncapsulationKey::new failed");
        black_box(ek);
    });

    // Encapsulate against an existing EK.
    let ek = EncapsulationKey::<MlKem768>::new(&ek_bytes).expect("EncapsulationKey::new failed");
    measure(
        "mlkem_encapsulate",
        iterations,
        read_cycles,
        cpu_mhz,
        || {
            let out = ek.encapsulate_deterministic(black_box(&m));
            black_box(out);
        },
    );

    bench_emit!("crypto=done iters={iterations}");
}

#[cfg(not(feature = "crypto-bench"))]
pub fn run(_iterations: u32, _read_cycles: fn() -> u32, _cpu_mhz: u32) {}

/// Runs the `body` `iterations` times, timing each run with the CPU cycle counter.
/// Each iteration emits a benchmarking output for computing statistics later.
#[cfg(feature = "crypto-bench")]
fn measure<F: FnMut()>(
    op: &str,
    iterations: u32,
    read_cycles: fn() -> u32,
    cpu_mhz: u32,
    mut body: F,
) {
    for i in 0..iterations {
        let start = read_cycles();
        body();
        let end = read_cycles();
        let cycles = end.wrapping_sub(start);
        bench_emit!("crypto=sample op={op} i={i} cycles={cycles} mhz={cpu_mhz}");
    }
}
