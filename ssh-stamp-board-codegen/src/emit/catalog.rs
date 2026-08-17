// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The rustdoc board catalog.
//!
//! Renders a table of every known board into a `board_catalog` module, so
//! `cargo doc` shows the pin map without anyone maintaining it by hand.

use std::fmt::Write as _;

use crate::error::Result;
use crate::load::Board;

/// A target-specific catalog column, inserted between `UART TX` and `URL`.
pub struct Column<E> {
    pub header: &'static str,
    pub cell: fn(&Board<E>) -> String,
}

/// Emit the `board_catalog` rustdoc table.
///
/// # Errors
///
/// Returns an error only if writing to the output string fails.
pub fn catalog<E>(out: &mut String, boards: &[Board<E>], extra: &[Column<E>]) -> Result<()> {
    let mut headers = String::from("| Board feature | UART RX | UART TX |");
    let mut rule = String::from("|---|---|---|");
    for column in extra {
        write!(headers, " {} |", column.header)?;
        rule.push_str("---|");
    }
    headers.push_str(" URL |");
    rule.push_str("---|");

    writeln!(
        out,
        "/// # Available boards\n///\n/// {headers}\n/// {rule}"
    )?;

    for b in boards {
        write!(
            out,
            "/// | `{}` | {} | {} |",
            b.feature, b.uart_rx, b.uart_tx
        )?;
        for column in extra {
            write!(out, " {} |", (column.cell)(b))?;
        }
        let url = match &b.url {
            Some(u) => format!("<{u}>"),
            None => "—".to_string(),
        };
        writeln!(out, " {url} |")?;
    }

    writeln!(out, "pub mod board_catalog {{}}\n")?;
    Ok(())
}
