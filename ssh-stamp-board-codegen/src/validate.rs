// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Cross-checking selected cargo features against the board files on disk.

use crate::error::{BuildError, Result};
use crate::load::Board;

/// Check that every selected `CARGO_FEATURE_BOARD_*` has a matching board
/// TOML, so a typo in `Cargo.toml` fails the build script with a clear
/// message rather than silently generating nothing.
///
/// # Errors
///
/// Returns [`BuildError::MissingBoardDef`] for the first selected board
/// feature with no corresponding `boards/*.toml`.
pub fn features<E>(boards: &[Board<E>]) -> Result<()> {
    for (key, _val) in std::env::vars() {
        if let Some(rest) = key.strip_prefix("CARGO_FEATURE_BOARD_") {
            let requested = format!("board-{}", rest.to_lowercase().replace('_', "-"));
            if boards.iter().any(|b| b.feature == requested) {
                continue;
            }
            return Err(BuildError::MissingBoardDef { feature: requested }.into());
        }
    }
    Ok(())
}
