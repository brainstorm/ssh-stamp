// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The build level information on boards for ssh-stamp.

use crate::results::CrateSize;
use crate::util::workspace_root;
use anyhow::{Context, Result};
use clap::builder::{PossibleValuesParser, TypedValueParser};
use ssh_stamp::config::UartPins;
use std::fs;
use std::path::PathBuf;
use xshell::{Shell, cmd};

/// A board the firmware can be built and flashed for.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Board {
    /// The board name.
    pub name: &'static str,
    /// The feature that selects the board's pins.
    pub feature: &'static str,
    /// The cargo package that builds this board's firmware.
    ///
    /// Not every board is an Espressif one: the BL616 port is a separate
    /// crate, with its own dependencies and its own linker arrangement.
    pub package: &'static str,
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

/// A bare chip with no board definition yet: the firmware binary would hit
/// the "No board feature selected" guard, so only the library is built.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Chip {
    /// The chip name, which is also the cargo feature that selects it.
    pub name: &'static str,
    /// The cargo package that builds this chip's firmware.
    pub package: &'static str,
    /// The rust target.
    pub target: &'static str,
    /// The toolchain that builds this target.
    pub toolchain: &'static str,
    /// For xtensa, adds `-Z build-std=core,alloc`.
    pub build_std: bool,
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
/// BL616 RAM, as the vendor linker script lays it out. Low and high
/// addresses, like the entries above -- not a base and a length.
///
/// 319 KB of general purpose RAM. A further 160 KB at `0x2301_0000` is reserved
/// for the `WiFi` blobs and is not counted here, because no ssh-stamp
/// allocation can go there.
const BL616_RAM: &[(u64, u64)] = &[(0x62FC_0400, 0x6301_0000)];

pub const BOARDS: &[Board] = &[
    Board {
        name: "esp32c5-devkitc",
        feature: "board-esp32c5-devkitc",
        package: "ssh-stamp-esp32",
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
        package: "ssh-stamp-esp32",
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
        package: "ssh-stamp-esp32",
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
        package: "ssh-stamp-esp32",
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
        // Sipeed M0S Dock. The WiFi comes from bl616-wifi, which links the
        // vendor's closed 802.11 blobs, so this target needs BL_SDK_BASE
        // pointing at a BouffaloSDK checkout at build time.
        name: "sipeed-m0s-dock",
        feature: "board-sipeed-m0s-dock",
        package: "ssh-stamp-bl616",
        soc: "bl616",
        // Hard float. The vendor archives are ilp32f, so an ilp32 target
        // (riscv32imac, as the ESP32-C6 uses) will not link against them.
        target: "riscv32imafc-unknown-none-elf",
        toolchain: "stable",
        build_std: false,
        riscv: true,
        ram: BL616_RAM,
        // No ESP-IDF partition table on this part; the boot ROM reads a
        // header written into the image by the post-processing step instead.
        partitions: None,
        max_flash_kib: None,
        max_ram_kib: None,
    },
    Board {
        name: "waveshare-esp32-s3-touch-lcd-43",
        feature: "board-waveshare-esp32-s3-touch-lcd-43",
        package: "ssh-stamp-esp32",
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

/// Every chip supported by the xtask. Chips targeted by a board are
/// still listed so the library remains buildable for custom boards or
/// future entries.
pub const CHIPS: &[Chip] = &[
    Chip {
        name: "esp32",
        package: "ssh-stamp-esp32",
        target: "xtensa-esp32-none-elf",
        toolchain: "esp",
        build_std: true,
    },
    Chip {
        name: "esp32c2",
        package: "ssh-stamp-esp32",
        target: "riscv32imc-unknown-none-elf",
        toolchain: "stable",
        build_std: false,
    },
    Chip {
        name: "esp32c3",
        package: "ssh-stamp-esp32",
        target: "riscv32imc-unknown-none-elf",
        toolchain: "stable",
        build_std: false,
    },
    Chip {
        name: "esp32c5",
        package: "ssh-stamp-esp32",
        target: "riscv32imac-unknown-none-elf",
        toolchain: "stable",
        build_std: false,
    },
    Chip {
        name: "esp32c6",
        package: "ssh-stamp-esp32",
        target: "riscv32imac-unknown-none-elf",
        toolchain: "stable",
        build_std: false,
    },
    Chip {
        name: "esp32c61",
        package: "ssh-stamp-esp32",
        target: "riscv32imac-unknown-none-elf",
        toolchain: "stable",
        build_std: false,
    },
    Chip {
        name: "esp32s2",
        package: "ssh-stamp-esp32",
        target: "xtensa-esp32s2-none-elf",
        toolchain: "esp",
        build_std: true,
    },
    Chip {
        name: "esp32s3",
        package: "ssh-stamp-esp32",
        target: "xtensa-esp32s3-none-elf",
        toolchain: "esp",
        build_std: true,
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

/// What a target name resolves to: a board builds the firmware binary, a
/// bare chip builds the library only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Board(&'static Board),
    Chip(&'static Chip),
}

impl Target {
    /// Looks up a board or chip by name, boards first.
    pub fn find(name: &str) -> Option<Self> {
        find(name)
            .map(Target::Board)
            .or_else(|| CHIPS.iter().find(|c| c.name == name).map(Target::Chip))
    }

    /// The target name.
    pub fn name(&self) -> &'static str {
        match self {
            Target::Board(board) => board.name,
            Target::Chip(chip) => chip.name,
        }
    }

    /// The toolchain that builds this target.
    pub fn toolchain(&self) -> &'static str {
        match self {
            Target::Board(board) => board.toolchain,
            Target::Chip(chip) => chip.toolchain,
        }
    }

    /// The directory where cargo puts the artifacts.
    pub fn target_dir(&self) -> PathBuf {
        match self {
            Target::Board(board) => board.target_dir(),
            Target::Chip(chip) => chip.target_dir(),
        }
    }

    /// The cargo arguments that select the targe triple, package,
    /// artifact kind and base feature.
    pub fn selection_args(&self) -> Vec<String> {
        let mut arguments = vec![
            "--target".into(),
            self.triple().into(),
            "-p".into(),
            self.package().into(),
        ];

        if let Target::Chip(_) = self {
            arguments.push("--lib".into());
        }

        arguments.extend([
            "--no-default-features".into(),
            "--features".into(),
            self.feature().into(),
        ]);

        if self.build_std() {
            arguments.push("-Z".into());
            arguments.push("build-std=core,alloc".into());
        }

        arguments
    }

    /// The rust target triple.
    fn triple(&self) -> &'static str {
        match self {
            Target::Board(board) => board.target,
            Target::Chip(chip) => chip.target,
        }
    }

    /// The cargo package that builds this target's firmware.
    fn package(&self) -> &'static str {
        match self {
            Target::Board(board) => board.package,
            Target::Chip(chip) => chip.package,
        }
    }

    /// The feature selecting the target.
    fn feature(&self) -> &'static str {
        match self {
            Target::Board(board) => board.feature,
            Target::Chip(chip) => chip.name,
        }
    }

    /// Adds `-Z build-std=core,alloc`.
    fn build_std(&self) -> bool {
        match self {
            Target::Board(board) => board.build_std,
            Target::Chip(chip) => chip.build_std,
        }
    }
}

impl Chip {
    /// The directory for artifacts.
    pub fn target_dir(&self) -> PathBuf {
        PathBuf::from("target").join("chips").join(self.name)
    }
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
            .join(self.package)
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
            self.package.into(),
            "--bin".into(),
            self.package.into(),
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

    /// The UART pins from the board's TOML definition.
    pub fn uart_pins(&self) -> Result<UartPins> {
        Ok(BoardToml::from_workspace(self.name)?.uarts_pins())
    }
}

/// The parts of a board definition TOML.
#[derive(serde::Deserialize)]
pub struct BoardToml {
    pins: BoardPins,
}

impl BoardToml {
    /// Parse the board toml from a string.
    pub fn parse_from_str(toml: &str) -> Result<BoardToml> {
        toml::from_str(toml).with_context(|| "could not parse".to_string())
    }

    /// Create the board configuration from the workspace definition.
    pub fn from_workspace(name: &str) -> Result<BoardToml> {
        let path = workspace_root()
            .join("ssh-stamp-esp32-boards")
            .join("boards")
            .join(format!("{name}.toml"));
        let definition = fs::read_to_string(&path)
            .with_context(|| format!("could not read {}", path.display()))?;

        Self::parse_from_str(&definition)
    }

    /// Get the UART pins for this board toml.
    pub fn uarts_pins(&self) -> UartPins {
        UartPins {
            rx: self.pins.uart_rx,
            tx: self.pins.uart_tx,
        }
    }
}

/// The pin section of a board definition TOML.
#[derive(serde::Deserialize)]
pub struct BoardPins {
    uart_rx: u8,
    uart_tx: u8,
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
    fn uart_pins_from_the_board_toml() {
        let board = find("esp32c6-devkitc").unwrap();
        assert_eq!(board.uart_pins().unwrap(), UartPins { rx: 10, tx: 11 });
    }

    #[test]
    fn selection_args() {
        let board = Target::find("esp32c6-devkitc").unwrap();
        assert_eq!(
            board.selection_args(),
            [
                "--target",
                "riscv32imac-unknown-none-elf",
                "-p",
                "ssh-stamp-esp32",
                "--no-default-features",
                "--features",
                "board-esp32c6-devkitc",
            ]
        );

        let chip = Target::find("esp32s3").unwrap();
        let args = chip.selection_args();
        assert!(args.contains(&"--lib".to_string()));
        assert!(args.ends_with(&["-Z".into(), "build-std=core,alloc".into()]));
        assert_eq!(Target::find("esp32c3"), Some(Target::Chip(&CHIPS[2])));
    }

    #[test]
    fn select_all() {
        let all = select(&[], true).into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(all, BOARDS.to_vec());
    }
}
