// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Utility functions.

use anyhow::Result;
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};
use xshell::Shell;

/// The workspace root directory, which is the parent of the xtask crate.
pub fn workspace_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
}

/// A shell with the workspace as the root, so xtask works from any directory.
pub fn shell() -> Result<Shell> {
    let sh = Shell::new()?;
    sh.change_dir(workspace_root());
    Ok(sh)
}

/// A helper function to call the `operation` until it succeeds or the `timeout` passes, sleeping
/// for the `interval` between attempts.
pub fn retry<T, E>(
    timeout: Duration,
    interval: Duration,
    mut operation: impl FnMut() -> Result<T, E>,
) -> Result<T, E> {
    let deadline = Instant::now() + timeout;
    loop {
        match operation() {
            Err(_) if Instant::now() < deadline => sleep(interval),
            result => return result,
        }
    }
}

/// A helper function to call the `condition` until it holds or the `timeout` passes, sleeping
/// for the `interval` between attempts.
pub fn retry_until(
    timeout: Duration,
    interval: Duration,
    mut condition: impl FnMut() -> bool,
) -> bool {
    retry(timeout, interval, || condition().then_some(()).ok_or(())).is_ok()
}
