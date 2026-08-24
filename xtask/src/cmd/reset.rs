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
    }

    eprintln!("=== reset complete, the device reboots with a fresh configuration ===");
    Ok(())
}

/// Espressif boards use espflash, anything else uses a debug probe.
pub fn default_mode(board: &Board) -> Mode {
    // As of now, this is only esp boards.
    if Chip::from_str(board.soc).is_ok() {
        Mode::Espflash
    } else {
        Mode::ProbeRs
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::find;

    #[test]
    fn reset() {
        let regions = erase_regions(find("esp32c6-devkitc").unwrap(), false).unwrap();
        let names = regions.iter().map(|r| r.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["app_config", "extra_data"]);

        assert_eq!(regions[0].offset, 0x9000);
        assert_eq!(regions[0].size, 0x2000);
        assert_eq!(regions[1].offset, 0x003d_0000);
        assert_eq!(regions[1].size, 0x0001_0000);

        let regions = erase_regions(find("esp32c6-devkitc").unwrap(), true).unwrap();
        let names = regions.iter().map(|r| r.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["app_config", "otadata", "extra_data"]);

        assert!(erase_regions(find("esp32-s2-saola").unwrap(), false).is_err());

        for board in board::BOARDS {
            assert_eq!(default_mode(board), Mode::Espflash);
        }
    }
}
