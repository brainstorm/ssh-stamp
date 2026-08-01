// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Errors surfaced by a BSP build script.

use std::fmt;
use std::path::PathBuf;

/// Boxed-error alias used throughout the codegen.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Errors a BSP build script can fail with.
#[derive(Debug)]
pub enum BuildError {
    /// A board feature was selected in Cargo.toml but no matching TOML file
    /// was found in `boards/`.
    MissingBoardDef { feature: String },
    /// A `boards/*.toml` file could not be read or parsed.
    BoardFile {
        path: PathBuf,
        source: Box<dyn std::error::Error>,
    },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBoardDef { feature } => write!(
                f,
                "Feature `{feature}` is enabled in Cargo.toml but no corresponding \
                 board definition file `boards/{feature}.toml` was found. \
                 Create it with a [pins] section containing uart_rx and uart_tx.",
            ),
            Self::BoardFile { path, source } => {
                write!(
                    f,
                    "Failed to process board file {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for BuildError {}
