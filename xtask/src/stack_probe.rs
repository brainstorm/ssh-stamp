// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The stack probe that measures stack usage on the board.

use crate::elf::StackRegion;
use crate::results::StackSnapshot;
use crate::util::retry;
use anyhow::{Context, Result};
use probe_rs::{MemoryInterface, Session, SessionConfig};
use probe_rs_espressif::register_plugin;
use std::sync::Once;
use std::time::Duration;

/// The word that is used for stack painting.
const STACK_PAINT: u32 = 0xA5A5_A5A5;

/// How long to wait for the core to halt.
const HALT_TIMEOUT: Duration = Duration::from_secs(3);

/// How long to keep retrying the probe after a reset.
const ATTACH_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to wait between attach attempts.
const ATTACH_RETRY: Duration = Duration::from_millis(500);

/// Must register the probe-rs espressif plugin.
///
/// See: <https://github.com/probe-rs/probe-rs/blob/v0.32.0/probe-rs-espressif/README.md>
fn register_espressif() {
    static ONCE: Once = Once::new();
    ONCE.call_once(register_plugin);
}

/// Attaches a probe-rs session to the `soc`.
pub fn attach_session(soc: &str) -> Result<Session> {
    register_espressif();

    // This needs to loop in order for the attach to persist across
    // resets, otherwise flaky benchmarking occurs.
    retry(ATTACH_TIMEOUT, ATTACH_RETRY, || {
        Session::auto_attach(soc, SessionConfig::default())
    })
    .with_context(|| format!("attaching to the {soc} debug link"))
}

/// A debug link to the board.
pub struct StackProbe {
    session: Session,
    region: StackRegion,
}

impl StackProbe {
    /// Attaches to the `soc` over any connected probe.
    pub fn attach(soc: &str, region: StackRegion) -> Result<StackProbe> {
        Ok(StackProbe {
            session: attach_session(soc)?,
            region,
        })
    }

    /// Paints the stack with [`STACK_PAINT`].
    pub fn paint(&mut self) -> Result<()> {
        let paint = vec![STACK_PAINT; self.region.words()];

        let mut core = self.session.core(0)?;
        core.reset_and_halt(HALT_TIMEOUT)?;
        core.write_32(self.region.floor, &paint)
            .context("painting the stack reservation")?;

        Ok(())
    }

    /// Releases the halted chip so it boots.
    pub fn run(&mut self) -> Result<()> {
        Ok(self.session.core(0)?.run()?)
    }

    /// Halts the chip and measures the stack usage.
    pub fn max_usage(&mut self) -> Result<StackSnapshot> {
        let mut words = vec![0u32; self.region.words()];

        let mut core = self.session.core(0)?;
        core.halt(HALT_TIMEOUT)?;
        core.read_32(self.region.floor, &mut words)
            .context("scanning the stack reservation")?;
        core.run()?;

        let untouched = words.iter().take_while(|w| **w == STACK_PAINT).count() as u64;
        Ok(StackSnapshot {
            label: "run".to_string(),
            max_bytes: self.region.start - (self.region.floor + untouched * 4),
            reserved_bytes: self.region.start - self.region.end,
        })
    }
}
