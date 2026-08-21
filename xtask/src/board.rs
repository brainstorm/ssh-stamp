// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The build level information on boards for ssh-stamp.

use crate::results::CrateSize;
use anyhow::{Context, Result};
use clap::builder::{PossibleValuesParser, TypedValueParser};
use std::path::PathBuf;
use xshell::{Shell, cmd};

/// A board the firmware can be built and flashed for.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Board {
    /// The board name.
    pub name: &'static str,
    /// The feature that selects the board's pins.
    pub feature: &'static str,
    /// The `SoC` for this board.
    pub soc: &'static str,
    /// The rust target.
    pub target: &'static str,
    /// The toolchain that builds this target.
    pub toolchain: &'static str,
    /// For xtensa, adds `-Z build-std=core,alloc`.
    pub build_std: bool,
    /// Set to `true` for RISC-V boards, `false` for Xtensa.
    pub riscv: bool,
    /// The inclusive start and exclusive end address windows for the internal RAM.
    pub ram: &'static [(u64, u64)],
    /// The partition table to build for.
    pub partitions: Option<&'static str>,
    /// The maximum flash in KiB before a failure.
    pub max_flash_kib: Option<u64>,
    /// The maximum RAM in KiB before a failure.
    pub max_ram_kib: Option<u64>,
}

/// The partition table to flash the firmware with.
const PARTITIONS: &str = "ssh-stamp-esp32/partitions.csv";

/// The cargo profile to build the firmware with.
pub const PROFILE: &str = "release";

/// The RAM regions that the esp32c5 declares. These values should be updated if esp-hal ever
/// updates the regions.
///
/// Sourced from: <https://github.com/esp-rs/esp-hal/blob/esp-hal-v1.1.1/esp-hal/ld/esp32c5/memory.x>
const ESP32C5_RAM: &[(u64, u64)] = &[(0x4080_0000, 0x4085_E5A0), (0x5000_0000, 0x5000_4000)];

/// The RAM regions that the esp32c6 declares. These values should be updated if esp-hal ever
/// updates the regions.
///
/// Sourced from: <https://github.com/esp-rs/esp-hal/blob/esp-hal-v1.1.1/esp-hal/ld/esp32c6/memory.x>
const ESP32C6_RAM: &[(u64, u64)] = &[(0x4080_0000, 0x4087_E610), (0x5000_0000, 0x5000_4000)];

/// The RAM regions that the esp32c61 declares. These values should be updated if esp-hal ever
/// updates the regions.
///
/// Sourced from: <https://github.com/esp-rs/esp-hal/blob/esp-hal-v1.1.1/esp-hal/ld/esp32c61/memory.x>
const ESP32C61_RAM: &[(u64, u64)] = &[(0x4080_0000, 0x4084_EA70)];

/// The RAM regions that the esp32s2 declares. These values should be updated if esp-hal ever
/// updates the regions.
///
/// Sourced from: <https://github.com/esp-rs/esp-hal/blob/esp-hal-v1.1.1/esp-hal/ld/esp32s2/memory.x>
const ESP32S2_RAM: &[(u64, u64)] = &[
    (0x3FF9_E000, 0x3FFA_0000),
    (0x3FFB_0000, 0x4000_0000),
    (0x4002_0000, 0x4004_E000),
    (0x4007_0000, 0x4007_2000),
    (0x5000_0000, 0x5000_2000),
];

/// The RAM regions that the esp32s3 declares. These values should be updated if esp-hal ever
/// updates the regions.
///
/// Sourced from: <https://github.com/esp-rs/esp-hal/blob/esp-hal-v1.1.1/esp-hal/ld/esp32s3/memory.x>
const ESP32S3_RAM: &[(u64, u64)] = &[
    (0x3FC8_8000, 0x3FCE_D710),
    (0x4037_0000, 0x403C_2000),
    (0x5000_0000, 0x5000_2000),
    (0x600F_E000, 0x6010_0000),
];

/// Every board supported by the xtask.
pub const BOARDS: &[Board] = &[
    Board {
        name: "esp32c5-devkitc",
        feature: "board-esp32c5-devkitc",
        soc: "esp32c5",
        target: "riscv32imac-unknown-none-elf",
        toolchain: "stable",
        build_std: false,
        riscv: true,
        ram: ESP32C5_RAM,
        partitions: Some(PARTITIONS),
        max_flash_kib: None,
        max_ram_kib: None,
    },
    Board {
        name: "esp32c6-devkitc",
        feature: "board-esp32c6-devkitc",
        soc: "esp32c6",
        target: "riscv32imac-unknown-none-elf",
        toolchain: "stable",
        build_std: false,
        riscv: true,
        ram: ESP32C6_RAM,
        partitions: Some(PARTITIONS),
        max_flash_kib: Some(1152),
        max_ram_kib: Some(240),
    },
    Board {
        name: "esp32c61-devkitc",
        feature: "board-esp32c61-devkitc",
        soc: "esp32c61",
        target: "riscv32imac-unknown-none-elf",
        toolchain: "stable",
        build_std: false,
        riscv: true,
        ram: ESP32C61_RAM,
        partitions: Some(PARTITIONS),
        max_flash_kib: None,
        max_ram_kib: None,
    },
    Board {
        name: "esp32-s2-saola",
        feature: "board-esp32-s2-saola",
        soc: "esp32s2",
        target: "xtensa-esp32s2-none-elf",
        toolchain: "esp",
        build_std: true,
        riscv: false,
        ram: ESP32S2_RAM,
        partitions: None,
        max_flash_kib: None,
        max_ram_kib: None,
    },
    Board {
        name: "waveshare-esp32-s3-touch-lcd-43",
        feature: "board-waveshare-esp32-s3-touch-lcd-43",
        soc: "esp32s3",
        target: "xtensa-esp32s3-none-elf",
        toolchain: "esp",
        build_std: true,
        riscv: false,
        ram: ESP32S3_RAM,
        partitions: Some(PARTITIONS),
        max_flash_kib: None,
        max_ram_kib: None,
    },
];

/// A struct representing the `cargo bloat` result.
#[derive(serde::Deserialize)]
pub struct Bloat {
    crates: Vec<CrateSize>,
}

impl Bloat {
    /// Parses a `cargo bloat --crates --message-format json` command.
    pub fn parse_bloat(json: &str) -> Result<Self> {
        serde_json::from_str(json).context("cargo bloat JSON")
    }

    /// Get the inner crate sizes.
    pub fn into_inner(self) -> Vec<CrateSize> {
        self.crates
    }
}

/// Looks up a board by name.
pub fn find(name: &str) -> Option<&'static Board> {
    BOARDS.iter().find(|b| b.name == name)
}

/// A clap value parser that resolves `--board` to the entry.
pub fn name_parser() -> impl TypedValueParser<Value = &'static Board> {
    PossibleValuesParser::new(BOARDS.iter().map(|b| b.name))
        .map(|name: String| find(&name).expect("only accepting args from here"))
}

/// Resolves a `--board` / `--all` selection.
pub fn select(boards: &[&'static Board], all: bool) -> Vec<&'static Board> {
    if all {
        return BOARDS.iter().collect();
    }
    boards.to_vec()
}

impl Board {
    /// The features for a build formatted as a `String`.
    pub fn features(&self, extra: &[&str]) -> String {
        let mut features = vec![self.feature];

        for extra_feature in extra {
            if !features.contains(extra_feature) {
                features.push(extra_feature);
            }
        }

        features.join(",")
    }

    /// The target subdirectory cargo writes artifacts to.
    fn profile_dir(profile: &str) -> &str {
        if profile == "dev" { "debug" } else { profile }
    }

    /// The directory where cargo puts the artifacts.
    pub fn target_dir(&self) -> PathBuf {
        PathBuf::from("target").join("boards").join(self.name)
    }

    /// The path to the firmware ELF for a profile.
    pub fn elf_path(&self, profile: &str) -> PathBuf {
        self.target_dir()
            .join(self.target)
            .join(Self::profile_dir(profile))
            .join("ssh-stamp-esp32")
    }

    /// The cargo arguments used by `build` and `bloat`.
    fn cargo_selection(&self, profile: &str, features: &str) -> Vec<String> {
        let mut arguments = vec![
            "--profile".into(),
            profile.into(),
            "--target".into(),
            self.target.into(),
            "--target-dir".into(),
            self.target_dir().display().to_string(),
            "-p".into(),
            "ssh-stamp-esp32".into(),
            "--bin".into(),
            "ssh-stamp-esp32".into(),
            "--no-default-features".into(),
            "--features".into(),
            features.into(),
        ];

        if self.build_std {
            arguments.push("-Z".into());
            arguments.push("build-std=core,alloc".into());
        }

        arguments
    }

    /// Builds the firmware for the board.
    pub fn build(
        &self,
        sh: &Shell,
        profile: &str,
        features: &str,
        env: &[(String, String)],
    ) -> Result<()> {
        let toolchain = format!("+{}", self.toolchain);
        let selection = self.cargo_selection(profile, features);

        eprintln!(
            "=== building {} (profile: {profile}, features: {features}) ===",
            self.name
        );

        let mut command = cmd!(sh, "cargo {toolchain} build {selection...}");
        for (key, value) in env {
            command = command.env(key, value);
        }

        command
            .run()
            .with_context(|| format!("cargo build for {} failed", self.name))?;

        Ok(())
    }

    /// Run `cargo bloat --crates` for the board.
    pub fn bloat(
        &self,
        sh: &Shell,
        profile: &str,
        features: &str,
        top: u32,
    ) -> Option<Vec<CrateSize>> {
        let toolchain = format!("+{}", self.toolchain);
        let selection = self.cargo_selection(profile, features);
        let top = top.to_string();

        let output = cmd!(
            sh,
            "cargo {toolchain} bloat {selection...} --crates --message-format json -n {top}"
        )
        .read();

        match output {
            Ok(json) => match Bloat::parse_bloat(&json) {
                Ok(crates) => Some(crates.into_inner()),
                Err(e) => {
                    eprintln!(
                        "warning: could not parse `cargo bloat` for {}: {e}",
                        self.name
                    );
                    None
                }
            },
            Err(e) => {
                eprintln!("warning: `cargo bloat` failed for {}: {e}", self.name);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_crates_json() {
        let json = serde_json::json!({
            "file-size": 123,
            "text-section-size": 90,
            "crates": [
                { "name": "ml_kem", "size": 40 },
                { "name": "std", "size": 20 },
            ],
        });
        let crates = Bloat::parse_bloat(&json.to_string()).unwrap().into_inner();
        assert_eq!(crates.len(), 2);
        assert_eq!(crates[0].name, "ml_kem");
        assert_eq!(crates[0].size_bytes, 40);
    }

    #[test]
    fn elf_path_profile_dir() {
        let board = find("esp32c6-devkitc").unwrap();
        assert!(board.elf_path("release").ends_with(
            "target/boards/esp32c6-devkitc/riscv32imac-unknown-none-elf/release/ssh-stamp-esp32"
        ));
        assert_ne!(
            board.elf_path("release"),
            find("esp32c5-devkitc").unwrap().elf_path("release")
        );
        assert!(board.elf_path("dev").to_string_lossy().contains("debug"));
    }

    #[test]
    fn ram_windows_are_sorted() {
        for board in BOARDS {
            for &(low, high) in board.ram {
                assert!(low < high);
            }
            for pair in board.ram.windows(2) {
                assert!(pair[0].1 <= pair[1].0);
            }
        }
    }

    #[test]
    fn features_build_set() {
        let board = find("esp32c6-devkitc").unwrap();
        assert_eq!(board.features(&[]), "board-esp32c6-devkitc");
        assert_eq!(
            board.features(&["mem-probe"]),
            "board-esp32c6-devkitc,mem-probe"
        );
        assert_eq!(
            board.features(&["board-esp32c6-devkitc"]),
            "board-esp32c6-devkitc"
        );
    }

    #[test]
    fn select_all() {
        let all = select(&[], true).into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(all, BOARDS.to_vec());
    }
}
