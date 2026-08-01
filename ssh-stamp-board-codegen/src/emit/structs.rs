// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `Board` trait and one struct per board.

use std::fmt::Write as _;

use crate::error::Result;
use crate::load::Board;

/// The `Board` trait, emitted immediately above the structs that implement
/// it.
///
/// It lives in the generated file rather than in each BSP crate's `lib.rs`
/// so that the `impl Board for …` blocks below — which name the trait
/// unqualified — always have it in scope, and so the four lines are not
/// copied into every BSP crate.
const BOARD_TRAIT: &str = r"/// Board identification trait.
///
/// Each board struct generated from `boards/*.toml` implements this trait.
/// The `NAME` const is the board's filename (without `.toml`), used for
/// boot-time logging.
pub trait Board {
    /// Human-readable board name (the `boards/*.toml` file stem).
    const NAME: &'static str;
}

";

/// Emit the `Board` trait definition.
pub fn board_trait(out: &mut String) {
    out.push_str(BOARD_TRAIT);
}

/// Emit one `pub struct` + `impl Board` per board, with a rustdoc link to
/// the board's documentation page when the TOML provided a `url`.
///
/// # Errors
///
/// Returns an error only if writing to the output string fails.
pub fn structs<E>(out: &mut String, boards: &[Board<E>]) -> Result<()> {
    for b in boards {
        let doc = match &b.url {
            Some(url) => format!("/// Board: {}.\n///\n/// <{url}>", b.name),
            None => format!("/// Board: {}.", b.name),
        };
        writeln!(
            out,
            "{doc}\npub struct {s};\nimpl Board for {s} {{\n    const NAME: &str = \"{n}\";\n}}",
            s = b.struct_name,
            n = b.name,
        )?;
        out.push('\n');
    }
    Ok(())
}
