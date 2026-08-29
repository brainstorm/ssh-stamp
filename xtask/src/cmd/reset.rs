// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! `xtask reset`
//!
//! Factory resets a board by erasing the non-volatile data partitions and
//! leaving the bootloader, partition table and firmware intact.

use crate::board::{self, Board};
use crate::device::Serial;
use crate::stack_probe;
use crate::util::workspace_root;
use anyhow::{Context, Result, bail};
use clap::{Args as ClapArgs, ValueEnum};
use esp_idf_part::{DataType, PartitionTable, SubType, Type};
use espflash::target::Chip;
use probe_rs::flashing::DownloadOptions;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use xshell::{Shell, cmd};

#[derive(ClapArgs)]
pub struct Args {
    /// Board to reset.
    #[arg(long, value_parser = board::name_parser())]
    board: &'static Board,
    /// The serial port for the espflash mode.
    #[arg(long)]
    port: Option<String>,
    /// Also erase the OTA state partition.
    #[arg(long)]
    erase_otadata: bool,
    /// The erase mode. Defaults to espflash for Espressif boards and
    /// probe-rs for anything else.
    #[arg(long, value_enum)]
    mode: Option<Mode>,
}

/// How the data partitions get erased.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Using espflash over the bootloader, Espressif chips only.
    Espflash,
    /// Over a debug probe, any chip probe-rs supports.
    ProbeRs,
    /// Using the vendor's `BLFlashCommand` over the ROM bootloader,
    /// Bouffalo chips only.
    BlFlash,
}

/// A flash region holding data that a reset erases.
#[derive(Debug)]
pub struct Region {
    name: String,
    offset: u32,
    size: u32,
}

pub fn run(args: &Args) -> Result<()> {
    let board = args.board;
    let regions = erase_regions(board, args.erase_otadata)?;
    if regions.is_empty() {
        bail!("{} has no data partitions to erase", board.name);
    }

    eprintln!("=== erasing bytes on {} ===", board.name);

    match args.mode.unwrap_or_else(|| default_mode(board)) {
        Mode::Espflash => espflash_erase(board, args.port.as_deref(), &regions)?,
        Mode::ProbeRs => probe_erase(board, &regions)?,
        Mode::BlFlash => blflash_erase(board, args.port.as_deref(), &regions)?,
    }

    eprintln!("=== reset complete, the device reboots with a fresh configuration ===");
    Ok(())
}

/// Espressif boards use espflash, Bouffalo ones the vendor flasher, anything
/// else a debug probe.
pub fn default_mode(board: &Board) -> Mode {
    if Chip::from_str(board.soc).is_ok() {
        Mode::Espflash
    } else if is_bouffalo(board) {
        Mode::BlFlash
    } else {
        Mode::ProbeRs
    }
}

/// Whether this board is one of Bouffalo's parts.
fn is_bouffalo(board: &Board) -> bool {
    board.soc.starts_with("bl")
}

/// The partition table path for the board.
pub fn partitions_csv(board: &Board) -> Result<PathBuf> {
    let Some(partitions) = board.partitions else {
        bail!("{} has no partition table", board.name);
    };
    Ok(workspace_root().join(partitions))
}

/// The data partitions a reset erases, sourced from the partition table.
pub fn erase_regions(board: &Board, erase_otadata: bool) -> Result<Vec<Region>> {
    if is_bouffalo(board) {
        if erase_otadata {
            // There is no otadata partition on this part. Which slot boots is
            // a field inside the partition table itself, and erasing that
            // leaves a board that Boot2 will not start at all.
            bail!(
                "{} keeps its OTA state inside the partition table, not in a partition; \
                 erasing it would leave the board unbootable",
                board.name
            );
        }
        return bl_erase_regions(board);
    }

    let path = partitions_csv(board)?;
    let csv = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let table = PartitionTable::try_from_str(&csv)
        .with_context(|| format!("parsing {}", path.display()))?;

    // Do not erase `phy_init`, and only erase the `otadata` if set, as `phy_init` is not
    // usually considered user data. It also matches existing conventions, e.g. see
    // https://github.com/espressif/esp-idf/blob/67c1de1eebe095d554d281952fde63c16ee2dca0/components/bootloader/Kconfig.projbuild#L199-L209
    Ok(table
        .partitions()
        .iter()
        .filter(|p| p.ty() == Type::Data)
        .filter(|p| p.subtype() != SubType::Data(DataType::Phy))
        .filter(|p| erase_otadata || p.subtype() != SubType::Data(DataType::Ota))
        .map(|p| Region {
            name: p.name(),
            offset: p.offset(),
            size: p.size(),
        })
        .collect())
}

/// Erases `regions` using espflash.
pub fn espflash_erase(board: &Board, port: Option<&str>, regions: &[Region]) -> Result<()> {
    let port = Serial::resolve_port(port)?;
    let shell = Shell::new()?;
    let soc = board.soc;
    let table = partitions_csv(board)?;

    let labels = regions
        .iter()
        .map(|region| region.name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    cmd!(
        shell,
        "espflash erase-parts --port {port} --chip {soc} --partition-table {table} {labels}"
    )
    .run()
    .with_context(|| format!("erasing the {labels} partitions"))?;

    Ok(())
}

/// Erases `regions` over a debug probe using probe-rs.
pub fn probe_erase(board: &Board, regions: &[Region]) -> Result<()> {
    let mut session = stack_probe::attach_session(board.soc)?;

    let mut loader = session.target().flash_loader();
    for region in regions {
        let data = vec![0xff_u8; usize::try_from(region.size)?];
        loader
            .add_data(u64::from(region.offset), &data)
            .with_context(|| format!("setting the {} partition to erase", region.name))?;
    }

    // The partition layout is much less coarse than the sector size of 64KiB on esp boards,
    // and the flash can only be erased in sectors. So first write the 0xFF bytes to the
    // board in the correct spots, and then do the actual commit with `keep_unwritten_bytes`.
    let mut options = DownloadOptions::new();
    options.keep_unwritten_bytes = true;
    loader
        .commit(&mut session, options)
        .context("erasing over debug probe")?;

    session.core(0)?.reset().context("rebooting the device")?;
    Ok(())
}

/// One entry of a Bouffalo `partition_cfg_*.toml`, as far as this needs it.
#[derive(serde::Deserialize)]
struct BlEntry {
    name: String,
    address0: u32,
    size0: u32,
}

/// The vendor's partition table description.
#[derive(serde::Deserialize)]
struct BlPartitionCfg {
    pt_entry: Vec<BlEntry>,
}

/// Partitions a factory reset erases on a Bouffalo board.
///
/// `DATA` only, and deliberately so. `factory` holds the RF calibration —
/// erase it and the radio stops transmitting sensibly — while `Boot2`, `FW`,
/// `mfg` and `media` are firmware rather than user data, and `PSM` and `KEY`
/// belong to the vendor stack rather than to ssh-stamp. `DATA` is where this
/// firmware's own configuration lives, so it is the whole of what a reset
/// has to remove.
const BL_DATA_PARTITIONS: &[&str] = &["DATA"];

/// Read the vendor partition table and pick the regions to erase.
///
/// The table is not in this repository: it is part of the `BouffaloSDK`, whose
/// flashing tool writes it to the board, so a reset has to read it from the
/// same place a build does.
fn bl_erase_regions(board: &Board) -> Result<Vec<Region>> {
    let path = bl_partition_cfg(board)?;
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let cfg: BlPartitionCfg =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;

    Ok(cfg
        .pt_entry
        .into_iter()
        .filter(|e| BL_DATA_PARTITIONS.contains(&e.name.as_str()))
        .map(|e| Region {
            name: e.name,
            offset: e.address0,
            size: e.size0,
        })
        .collect())
}

/// Locate the board's partition table inside the SDK.
///
/// `BL_SDK_BASE` and `BL616_BOARD` are the same two variables the firmware
/// build uses, so a checkout that can build can also reset.
fn bl_partition_cfg(board: &Board) -> Result<PathBuf> {
    let sdk = PathBuf::from(std::env::var("BL_SDK_BASE").context(
        "BL_SDK_BASE is not set, and the partition table for this board lives in the \
         BouffaloSDK; point it at a checkout",
    )?);
    let sdk_board = std::env::var("BL616_BOARD").unwrap_or_else(|_| "bl616dk".to_string());
    let dir = sdk.join("bsp/board").join(&sdk_board).join("config");

    // 4 MB is the layout this board ships with; fall back to whatever single
    // table the board directory has, rather than guessing between several.
    let four_meg = dir.join("partition_cfg_4M.toml");
    if four_meg.is_file() {
        return Ok(four_meg);
    }
    let mut found: Vec<PathBuf> = fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "toml")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("partition_cfg_"))
        })
        .collect();
    found.sort();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => bail!("no partition_cfg_*.toml in {}", dir.display()),
        _ => bail!(
            "several partition tables in {}; set BL616_BOARD to the one {} uses",
            dir.display(),
            board.name
        ),
    }
}

/// Erases `regions` with the vendor flasher over the ROM bootloader.
///
/// One invocation per region: the tool takes a single address range, and the
/// data partitions are not contiguous with each other on every layout.
pub fn blflash_erase(board: &Board, port: Option<&str>, regions: &[Region]) -> Result<()> {
    let sdk = PathBuf::from(
        std::env::var("BL_SDK_BASE").context("BL_SDK_BASE is not set; point it at a checkout")?,
    );
    let tool = sdk.join("tools/bflb_tools/bouffalo_flash_cube/BLFlashCommand-ubuntu");
    if !tool.is_file() {
        bail!("{} not found", tool.display());
    }

    let port = Serial::resolve_port(port)?;
    let shell = Shell::new()?;
    let soc = board.soc;
    let tool = tool.display().to_string();

    eprintln!("hold BOOT and tap RST first if the board is not already in the ROM bootloader");

    for region in regions {
        // The tool takes an inclusive range, and erases whole sectors around
        // whatever it is given.
        let start = format!("{:#x}", region.offset);
        let end = format!("{:#x}", region.offset + region.size - 1);
        eprintln!("erasing {} ({start}..={end})", region.name);
        cmd!(
            shell,
            "{tool} --interface=uart --port={port} --chipname={soc} --baudrate=2000000
             --flash --erase --start={start} --end={end}"
        )
        .run()
        .with_context(|| {
            format!(
                "erasing the {} partition; a handshake failure here means the board is \
                 running its firmware rather than sitting in the ROM bootloader — hold \
                 BOOT, tap RST, and try again",
                region.name
            )
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::find;

    /// The Bouffalo path reads its table out of the SDK, so this only runs
    /// where a checkout is configured — the same condition the firmware
    /// build has.
    #[test]
    fn bl616_reset_erases_only_the_data_partition() {
        if std::env::var_os("BL_SDK_BASE").is_none() {
            eprintln!("skipped: BL_SDK_BASE is not set");
            return;
        }
        let board = find("sipeed-m0s-dock").unwrap();
        assert_eq!(default_mode(board), Mode::BlFlash);

        let regions = erase_regions(board, false).unwrap();
        let names = regions.iter().map(|r| r.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["DATA"]);

        // And it is where the firmware was told the config lives, or a reset
        // would erase a region the board never writes.
        assert_eq!(format!("{:#X}", regions[0].offset), "0x3F3000");
        assert!(regions[0].size > 0);

        // Erasing the OTA state is not a thing that can be done here.
        assert!(erase_regions(board, true).is_err());
    }

    #[test]
    fn reset() {
        let regions = erase_regions(find("esp32c6-devkitc").unwrap(), false).unwrap();
        let names = regions.iter().map(|r| r.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["app_config", "extra_data"]);

        assert_eq!(regions[0].offset, 0x9000);
        assert_eq!(regions[0].size, 0x2000);
        assert_eq!(regions[1].offset, 0x003d_0000);
        assert_eq!(regions[1].size, 0x0001_0000);
    }
}
