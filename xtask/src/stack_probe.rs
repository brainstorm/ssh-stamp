// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The stack probe that measures stack usage on the board.

use crate::elf::StackRegion;
use crate::results::StackSnapshot;
use anyhow::{Context, Result};
use probe_rs::{MemoryInterface, Session, SessionConfig};
use std::time::Duration;

/// The word that is used for stack painting.
const STACK_PAINT: u32 = 0xA5A5_A5A5;

/// How long to wait for the core to halt.
const HALT_TIMEOUT: Duration = Duration::from_secs(3);

/// A debug link to the board.
pub struct StackProbe {
    session: Session,
    region: StackRegion,
}

impl StackProbe {
    /// Attaches to the `soc` over any connected probe.
    pub fn attach(soc: &str, region: StackRegion) -> Result<StackProbe> {
        let session = Session::auto_attach(soc, SessionConfig::default())
            .with_context(|| format!("attaching to the {soc} debug link"))?;

        Ok(StackProbe { session, region })
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
