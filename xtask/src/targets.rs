// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build registry: platforms and chips from `xtask/targets.toml`, boards
//! discovered by scanning each platform's `boards/*.toml` directory.
//!
//! Keeping board discovery filesystem-driven means adding a board is still
//! "drop in a TOML + add the cargo feature" — the build tool needs no edit.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

/// A cargo package producing firmware for one MCU family / vendor.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Platform {
    /// Cargo package to build (`-p`).
    pub package: String,
    /// Binary target inside that package (`--bin`).
    pub bin: String,
    /// Directory of board definitions, relative to the workspace root.
    pub boards: String,
    /// Prefix turning a board name into its cargo feature.
    pub board_feature_prefix: String,
    /// Board used by jobs that need one target rather than all of them.
    pub default_board: String,
    /// Library crates `cargo xtask doc` renders for this platform.
    pub doc_packages: Vec<String>,
}

/// One MCU: the Rust target triple plus how it has to be built.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Chip {
    /// Key into [`Registry::platforms`].
    pub platform: String,
    /// Rust target triple.
    pub target: String,
    /// Rustup toolchain override (e.g. `esp` for Xtensa), if any.
    #[serde(default)]
    pub toolchain: Option<String>,
    /// Whether the target needs `-Z build-std` (no prebuilt core).
    #[serde(default)]
    pub build_std: bool,
    /// Cargo profile override; defaults to `release`.
    #[serde(default)]
    pub profile: Option<String>,
}

/// Settings for jobs that need one representative target.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Defaults {
    /// Crates that build for the host rather than for a chip.
    pub host_packages: Vec<String>,
    /// Toolchain used when a chip does not name its own.
    #[serde(default)]
    pub toolchain: Option<String>,
}

/// Target name standing for "the host-side crates" rather than a chip.
pub const HOST: &str = "host";

/// A concrete PCB, discovered from a platform's boards directory.
#[derive(Debug)]
pub struct Board {
    pub name: String,
    pub chip: String,
    /// Extra cargo features this board always needs (e.g. `can` on a board
    /// whose whole point is the CAN transceiver).
    pub features: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    platforms: BTreeMap<String, Platform>,
    chips: BTreeMap<String, Chip>,
    defaults: Defaults,
}

/// The `[build]` section of a `boards/*.toml`. Every other section belongs
/// to the BSP build script and is ignored here.
#[derive(Debug, Deserialize)]
struct BoardFile {
    build: Option<BoardBuild>,
}

#[derive(Debug, Deserialize)]
struct BoardBuild {
    chip: String,
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Debug)]
pub struct Registry {
    pub platforms: BTreeMap<String, Platform>,
    pub chips: BTreeMap<String, Chip>,
    pub defaults: Defaults,
    pub boards: BTreeMap<String, Board>,
}

impl Registry {
    /// Parse `xtask/targets.toml` and scan every platform for boards.
    ///
    /// # Errors
    ///
    /// Returns a message if the registry or any board file is missing,
    /// unreadable, malformed, or refers to an unknown platform/chip.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("xtask/targets.toml");
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let file: RegistryFile =
            toml::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;

        let mut boards = BTreeMap::new();
        for (platform_name, platform) in &file.platforms {
            let dir = root.join(&platform.boards);
            for (name, board) in scan_boards(&dir)? {
                let chip = file.chips.get(&board.chip).ok_or_else(|| {
                    format!(
                        "board `{name}` wants chip `{}`, which is not in targets.toml",
                        board.chip
                    )
                })?;
                if &chip.platform != platform_name {
                    return Err(format!(
                        "board `{name}` lives in platform `{platform_name}` but its chip \
                         `{}` belongs to platform `{}`",
                        board.chip, chip.platform
                    ));
                }
                boards.insert(name, board);
            }
        }

        for (name, chip) in &file.chips {
            if !file.platforms.contains_key(&chip.platform) {
                return Err(format!(
                    "chip `{name}` refers to unknown platform `{}`",
                    chip.platform
                ));
            }
        }

        Ok(Registry {
            platforms: file.platforms,
            chips: file.chips,
            defaults: file.defaults,
            boards,
        })
    }

    /// Look up the platform a chip belongs to.
    ///
    /// # Errors
    ///
    /// Returns a message if the chip's platform is missing from the registry.
    pub fn platform_of(&self, chip: &Chip) -> Result<&Platform, String> {
        self.platforms
            .get(&chip.platform)
            .ok_or_else(|| format!("unknown platform `{}`", chip.platform))
    }

    /// The toolchain to build `chip` with: its own, else the default.
    pub fn toolchain_for<'a>(&'a self, chip: &'a Chip) -> Option<&'a str> {
        chip.toolchain
            .as_deref()
            .or(self.defaults.toolchain.as_deref())
    }

    /// The toolchain for host-side jobs (fmt, host tests).
    pub fn host_toolchain(&self) -> Option<&str> {
        self.defaults.toolchain.as_deref()
    }
}

fn scan_boards(dir: &Path) -> Result<Vec<(String, Board)>, String> {
    if !dir.is_dir() {
        return Err(format!("boards directory {} not found", dir.display()));
    }

    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    entries.sort();

    let mut boards = Vec::new();
    for path in entries {
        let name = path
            .file_stem()
            .ok_or_else(|| format!("board file {} has no name", path.display()))?
            .to_string_lossy()
            .into_owned();

        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let file: BoardFile =
            toml::from_str(&text).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;

        // A board without a [build] section is a BSP-only entry: it has pins
        // but nothing that says which chip to compile for, so it cannot be a
        // build target. Flag it rather than silently skipping it.
        let build = file.build.ok_or_else(|| {
            format!(
                "{} has no [build] section; add `chip = \"<chip>\"` so xtask \
                 knows how to build it",
                path.display()
            )
        })?;

        boards.push((
            name.clone(),
            Board {
                name,
                chip: build.chip,
                features: build.features,
            },
        ));
    }
    Ok(boards)
}
