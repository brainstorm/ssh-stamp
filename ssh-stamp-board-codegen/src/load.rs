// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Discovering and parsing `boards/*.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::error::{BuildError, Result};
use crate::naming;

/// A board definition, ready for codegen.
///
/// `E` is the target-specific extension: whatever extra tables and keys that
/// MCU family's BSP cares about, deserialized from the same document (see
/// [`load_boards_from`]).
#[derive(Debug)]
pub struct Board<E> {
    /// File stem, e.g. `esp32c6-devkitc`.
    pub name: String,
    /// Generated type name, e.g. `Esp32c6Devkitc`.
    pub struct_name: String,
    /// Cargo feature, e.g. `board-esp32c6-devkitc`.
    pub feature: String,
    /// Optional documentation URL for the PCB.
    pub url: Option<String>,
    pub uart_rx: u8,
    pub uart_tx: u8,
    /// Target-specific extension.
    pub ext: E,
}

/// The part of a board TOML every target has.
#[derive(Debug, Deserialize)]
struct Common {
    url: Option<String>,
    pins: CommonPins,
}

#[derive(Debug, Deserialize)]
struct CommonPins {
    uart_rx: u8,
    uart_tx: u8,
}

/// Load every `boards/*.toml` under `dir`, sorted by filename.
///
/// Each file is parsed twice: once into the common schema above, once into
/// the caller's `E`. Serde ignores keys a struct does not name, so an
/// extension can pick up both extra keys *inside* `[pins]` and whole extra
/// tables without any coordination with the common schema. Parsing twice
/// with `toml::from_str` (rather than going through `toml::Value`) keeps
/// serde's line/column spans in the error messages; board files are tiny, so
/// the cost is nil.
///
/// # Errors
///
/// Returns [`BuildError::BoardFile`] if a file cannot be read or parsed.
pub fn load_boards_from<E: DeserializeOwned>(dir: &Path) -> Result<Vec<Board<E>>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut paths: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();

    paths.iter().map(|p| parse_board(p)).collect()
}

fn parse_board<E: DeserializeOwned>(path: &Path) -> Result<Board<E>> {
    let stem = path
        .file_stem()
        .ok_or("board file has no stem")?
        .to_string_lossy()
        .to_string();

    let at = |source: Box<dyn std::error::Error>| BuildError::BoardFile {
        path: path.to_path_buf(),
        source,
    };

    let content = fs::read_to_string(path).map_err(|e| at(Box::new(e)))?;
    let common: Common = toml::from_str(&content).map_err(|e| at(Box::new(e)))?;
    let ext: E = toml::from_str(&content).map_err(|e| at(Box::new(e)))?;

    Ok(Board {
        struct_name: naming::to_pascal_case(&stem),
        feature: naming::feature_name(&stem),
        name: stem,
        url: common.url,
        uart_rx: common.pins.uart_rx,
        uart_tx: common.pins.uart_tx,
        ext,
    })
}
