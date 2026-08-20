// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! RNG for the RP2350, backed by the on-chip TRNG.
//!
//! The SSH host key and the first-boot secrets are minted through
//! `getrandom`, so this has to be a real entropy source: the RP2350's TRNG
//! block (ring oscillator, sampled and post-processed in hardware,
//! `CryptoRng` in `embassy-rp`), never the plain `RoscRng` counter.
//!
//! # Why a pool instead of reading the TRNG directly
//!
//! `getrandom` is synchronous, so the obvious implementation calls
//! `Trng::blocking_fill_bytes`. That hangs the firmware: embassy-rp's
//! blocking path spins on the busy flag with no yield and no timeout, so on
//! a cooperative executor a TRNG that has not warmed up starves every other
//! task — USB and its log channel included, which is why the board looks
//! dead rather than slow. The chip is worst at this right after power-on,
//! when it repeatedly fails its autocorrelation test, and that is exactly
//! when the first host key is generated.
//!
//! So [`entropy_task`] is the sole owner of the TRNG and reads it
//! asynchronously into a pool, which synchronous callers drain. Bytes are
//! raw TRNG output; an empty pool returns an error rather than blocking,
//! because a failed handshake is recoverable and a wedged board is not.
//!
//! # Wiring this into `getrandom`
//!
//! getrandom 0.4 selects the `custom` backend per target via
//! `--cfg getrandom_backend="custom"` (set in `.cargo/config.toml` for
//! `thumbv8m.main-none-eabihf`) and links an `extern "Rust"` symbol that
//! must be defined exactly once in the program. Defining it needs `unsafe`,
//! so it lives in the port binary — where getrandom's own docs put it — and
//! forwards to [`getrandom_fill_bytes`], keeping this crate
//! `#![forbid(unsafe_code)]`.

use core::cell::RefCell;

use embassy_rp::peripherals::TRNG;
use embassy_rp::trng::Trng;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer, with_timeout};
use log::{debug, info, warn};
use ssh_stamp_hal::HalError;

/// Bytes of TRNG output held ready for synchronous callers.
///
/// A host key needs 32, a key exchange a little more. This covers a burst of
/// connections without the refill task having to keep up in real time.
const POOL_BYTES: usize = 1024;

/// Bytes per refill attempt: exactly one TRNG generation (192 bits).
///
/// Asking for more makes one attempt span several generations, and since a
/// timeout discards the whole attempt, a chip that is slow to warm up then
/// never completes one — the pool stays empty while each try throws away its
/// progress.
const REFILL_CHUNK: usize = 24;

/// Ceiling for one TRNG read. Generous because the first generations after
/// power-on are slow; waiting costs nothing in a background task.
const TRNG_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Entropy to bank before boot continues past [`prime_pool`].
const PRIME_BYTES: usize = 256;

/// Ceiling on that boot-time wait.
const PRIME_TIMEOUT: Duration = Duration::from_secs(30);

struct Pool {
    buf: [u8; POOL_BYTES],
    /// Number of valid bytes, stored at the front of `buf`.
    len: usize,
}

impl Pool {
    const fn new() -> Self {
        Self {
            buf: [0; POOL_BYTES],
            len: 0,
        }
    }

    /// Takes `out.len()` bytes, or nothing if the pool is short.
    ///
    /// All-or-nothing so a caller never receives a partly-random buffer,
    /// which would be worse than a clean failure.
    fn take(&mut self, out: &mut [u8]) -> bool {
        if out.len() > self.len {
            return false;
        }
        let start = self.len - out.len();
        out.copy_from_slice(&self.buf[start..self.len]);
        // Consumed bytes are cleared, not just unreferenced: this buffer
        // holds key material until it is overwritten.
        self.buf[start..self.len].fill(0);
        self.len = start;
        true
    }

    fn add(&mut self, bytes: &[u8]) -> usize {
        let room = POOL_BYTES - self.len;
        let n = room.min(bytes.len());
        self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
        n
    }
}

static POOL: Mutex<CriticalSectionRawMutex, RefCell<Pool>> = Mutex::new(RefCell::new(Pool::new()));

/// Keeps the entropy pool topped up. Spawn once, early.
///
/// Owns the TRNG for the lifetime of the program; nothing else may read it,
/// because every other path into the chip is a blocking one.
#[embassy_executor::task]
pub async fn entropy_task(mut trng: Trng<'static, TRNG>) -> ! {
    let mut chunk = [0u8; REFILL_CHUNK];
    loop {
        if POOL.lock(|p| POOL_BYTES - p.borrow().len) < REFILL_CHUNK {
            Timer::after(Duration::from_millis(100)).await;
            continue;
        }

        if with_timeout(TRNG_READ_TIMEOUT, trng.fill_bytes(&mut chunk))
            .await
            .is_err()
        {
            warn!("TRNG read timed out; entropy pool is not refilling");
            Timer::after(Duration::from_millis(500)).await;
            continue;
        }

        let level = POOL.lock(|p| {
            let mut p = p.borrow_mut();
            p.add(&chunk);
            p.len
        });
        chunk.fill(0);
        debug!("entropy pool at {level} bytes");
    }
}

/// Fill `buf` with entropy from the pool.
///
/// # Errors
///
/// Returns [`HalError::Rng`] if the pool does not currently hold enough
/// bytes. Never blocks.
pub fn fill_bytes(buf: &mut [u8]) -> Result<(), HalError> {
    if POOL.lock(|p| p.borrow_mut().take(buf)) {
        return Ok(());
    }
    // Loud, because a caller deep inside an SSH handshake simply drops the
    // connection: silent failure looks like an unexplained reset from the
    // client's side.
    warn!(
        "entropy pool short: {} bytes requested, {} available",
        buf.len(),
        pool_level()
    );
    Err(HalError::Rng)
}

/// Safe half of the `getrandom` custom backend.
///
/// The binary's `__getrandom_v03_custom` shim forwards here; see the module
/// docs for why the split exists.
///
/// # Errors
///
/// Returns an error if the pool is short — see the module docs for why this
/// fails instead of waiting.
pub fn getrandom_fill_bytes(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    fill_bytes(buf).map_err(|_| getrandom::Error::UNEXPECTED)
}

/// Bytes currently available in the pool.
#[must_use]
pub fn pool_level() -> usize {
    POOL.lock(|p| p.borrow().len)
}

/// Waits for the pool to bank [`PRIME_BYTES`], or [`PRIME_TIMEOUT`] to pass,
/// reporting the outcome either way.
///
/// A key exchange needs more entropy than a single TRNG generation yields,
/// so an SSH server started against a nearly-empty pool produces handshakes
/// that die partway with nothing to show for it. Boot waits here instead.
pub async fn prime_pool() {
    let deadline = embassy_time::Instant::now() + PRIME_TIMEOUT;
    loop {
        let level = pool_level();
        if level >= PRIME_BYTES {
            info!("entropy pool ready ({level} bytes)");
            return;
        }
        if embassy_time::Instant::now() >= deadline {
            warn!("entropy pool only reached {level}/{PRIME_BYTES} bytes; SSH may fail");
            return;
        }
        Timer::after(Duration::from_millis(200)).await;
    }
}
