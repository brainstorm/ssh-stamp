// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! RNG for the RP2350, backed by the on-chip TRNG.
//!
//! `ssh-stamp` mints the SSH host key and the first-boot secrets through
//! `getrandom`, so a real entropy source must be registered before the
//! config is loaded — same contract as the ESP port, which goes out of its
//! way to enable true RNG for exactly this reason.
//!
//! We use the RP2350's TRNG block (ring oscillator sampled and
//! post-processed in hardware; `embassy-rp` marks it `CryptoRng`) rather
//! than the plain `RoscRng` counter, which is not an entropy source
//! suitable for key material.
//!
//! # Why a pool instead of reading the TRNG directly
//!
//! `getrandom` is synchronous, so the obvious implementation calls
//! `Trng::blocking_fill_bytes`. That turned out to hang the whole firmware.
//! embassy-rp's blocking path is
//!
//! ```text
//! while trng_busy_register.read().trng_busy() {}
//! ```
//!
//! with no yield and no timeout, and on a cooperative executor a TRNG that
//! has not warmed up starves every other task — USB, Ethernet, timers, the
//! lot. The board looks dead rather than slow, and it took a long time to
//! find because the log channel dies with everything else.
//!
//! The chip is worst at this immediately after power-on, when it fails its
//! autocorrelation test repeatedly, which is exactly when the first host key
//! is generated.
//!
//! So the TRNG is read only from [`entropy_task`], asynchronously, and its
//! output is kept in a pool that the synchronous callers drain. Bytes are
//! raw TRNG output as before — no new construction is introduced here — and
//! an empty pool returns an error rather than blocking. A failed handshake
//! is recoverable; a wedged board is not.

use core::cell::RefCell;

use embassy_rp::peripherals::TRNG;
use embassy_rp::trng::Trng;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer, with_timeout};
use getrandom::register_custom_getrandom;
use log::{debug, info, warn};
use ssh_stamp_hal::{HalError, RngHal};

/// Bytes of TRNG output held ready for synchronous callers.
///
/// A host key needs 32; a key exchange a little more. This covers a burst
/// of connections without the refill task having to keep up in real time.
const POOL_BYTES: usize = 1024;

/// Bytes per refill attempt.
///
/// Exactly one TRNG generation (192 bits). Asking for more makes a single
/// attempt span several generations, and since a timeout discards the whole
/// attempt, a chip that is slow to warm up then never completes one — the
/// pool stays empty forever while each try throws away its progress.
const REFILL_CHUNK: usize = 24;

/// Ceiling for one TRNG read (one generation).
///
/// Generous because the first generations after power-on are slow: the
/// hardware retries its autocorrelation test while the ring oscillator
/// settles. Waiting costs nothing here — this is a background task.
const TRNG_READ_TIMEOUT: Duration = Duration::from_secs(10);

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

register_custom_getrandom!(rp2350_getrandom_custom_func);

/// Keeps the entropy pool topped up. Spawn once, early.
///
/// Owns the TRNG for the lifetime of the program; nothing else may read it,
/// because every other path into the chip is a blocking one.
#[embassy_executor::task]
pub async fn entropy_task(mut trng: Trng<'static, TRNG>) -> ! {
    let mut chunk = [0u8; REFILL_CHUNK];
    let mut primed = false;
    loop {
        let space = POOL.lock(|p| POOL_BYTES - p.borrow().len);
        if space < REFILL_CHUNK {
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
        // The first fill is the one worth seeing: it is the difference
        // between a board that can complete a key exchange and one that
        // cannot, and it is otherwise invisible until SSH fails.
        if !primed && level >= REFILL_CHUNK {
            info!("entropy pool primed ({level} bytes)");
            primed = true;
        } else {
            debug!("entropy pool at {level} bytes");
        }
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
        Ok(())
    } else {
        warn!("entropy pool exhausted, {} bytes requested", buf.len());
        Err(HalError::Rng)
    }
}

/// RP2350 RNG handle.
pub struct Rp2350Rng;

impl Rp2350Rng {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for Rp2350Rng {
    fn default() -> Self {
        Self::new()
    }
}

impl RngHal for Rp2350Rng {
    async fn fill_bytes(&mut self, buf: &mut [u8]) -> Result<(), HalError> {
        // Async callers can afford to wait for a refill rather than fail.
        for _ in 0..20 {
            if POOL.lock(|p| p.borrow_mut().take(buf)) {
                return Ok(());
            }
            Timer::after(Duration::from_millis(50)).await;
        }
        Err(HalError::Rng)
    }
}

/// `getrandom` backend.
///
/// # Errors
///
/// Returns an error if the pool is empty — see the module docs for why this
/// fails instead of waiting.
pub fn rp2350_getrandom_custom_func(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    if POOL.lock(|p| p.borrow_mut().take(buf)) {
        Ok(())
    } else {
        // Loud, because the caller is usually deep inside an SSH handshake
        // and simply drops the connection: silent failure here looks like
        // an unexplained reset from the client's side.
        warn!(
            "getrandom: pool short, {} bytes requested, {} available",
            buf.len(),
            pool_level()
        );
        Err(getrandom::Error::UNEXPECTED)
    }
}

/// Bytes currently available in the pool.
#[must_use]
pub fn pool_level() -> usize {
    POOL.lock(|p| p.borrow().len)
}

/// Waits until the pool holds at least `want` bytes, or `timeout` passes.
///
/// A key exchange needs more entropy than a single TRNG generation yields,
/// so starting the SSH server against a nearly-empty pool produces
/// handshakes that die partway with nothing to show for it. Priming first
/// turns that into a bounded wait at boot.
///
/// Returns the level reached.
pub async fn prime_pool(want: usize, timeout: Duration) -> usize {
    let deadline = embassy_time::Instant::now() + timeout;
    loop {
        let level = pool_level();
        if level >= want || embassy_time::Instant::now() >= deadline {
            return level;
        }
        Timer::after(Duration::from_millis(200)).await;
    }
}
