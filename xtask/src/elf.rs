// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Measures the linked firmware using [`espflash`] and [`object`].

use anyhow::{Context, Result, anyhow, bail};
use espflash::flasher::{FlashData, FlashSettings};
use espflash::image_format::idf::IdfBootloaderFormat;
use espflash::target::{Chip, XtalFrequency};
use object::{File, Object, ObjectSection, ObjectSegment, ObjectSymbol};
use std::path::Path;
use std::str::FromStr;

use crate::board::Board;

/// The memory footprint of a firmware.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_field_names)]
pub struct Footprint {
    /// The size of the ESP-IDF application image itself, headers and padding included.
    pub flash_b: u64,
    /// The size of the internal RAM with every segment in the RAM windows.
    pub ram_b: u64,
    /// The RAM reserved for the stack by the linker.
    pub stack_reserved_b: u64,
}

impl Footprint {
    /// Measures the `elf` footprint for `board`.
    pub fn new(elf: &Path, board: &Board) -> Result<Footprint> {
        let data = read(elf)?;
        let file = File::parse(&*data).with_context(|| format!("parsing {}", elf.display()))?;
        let chip =
            Chip::from_str(board.soc).map_err(|_| anyhow!("unknown chip `{}`", board.soc))?;

        let (ram_b, stack_reserved_b) = Self::memory(&file, board, chip, elf)?;

        Ok(Footprint {
            flash_b: Self::image_size(&data, board, chip, elf)?,
            ram_b,
            stack_reserved_b,
        })
    }

    /// The size of the application image for this elf.
    pub fn image_size(elf: &[u8], board: &Board, chip: Chip, path: &Path) -> Result<u64> {
        let flash_data = FlashData::new(
            FlashSettings::default(),
            0,
            None,
            chip,
            XtalFrequency::default(),
        );

        let image = IdfBootloaderFormat::new(
            elf,
            &flash_data,
            board.partitions.map(Path::new),
            None,
            None,
            None,
        )
        .with_context(|| {
            format!(
                "building {} application image using {}",
                board.name,
                path.display()
            )
        })?;

        Ok(image.ota_segments().map(|s| u64::from(s.size())).sum())
    }

    /// The internal RAM used by the firmware, and the stack reserved bytes specified by the linker.
    pub fn memory(file: &File, board: &Board, chip: Chip, elf: &Path) -> Result<(u64, u64)> {
        let segments: Vec<_> = file
            .segments()
            .map(|segment| {
                let (_, file_size) = segment.file_range();
                (segment.address(), file_size, segment.size())
            })
            .collect();

        Self::memory_of(
            &segments,
            Self::stack_span(file, elf)?,
            Self::section_span(file, ".rwdata_dummy"),
            Self::section_span(file, ".rtc_fast.dummy"),
            board,
            chip,
        )
    }

    /// The RAM and stack reserved bytes from the extracted `segments`.
    pub fn memory_of(
        segments: &[(u64, u64, u64)],
        stack_span: (u64, u64),
        rwdata_dummy: Option<(u64, u64)>,
        rtc_fast_dummy: Option<(u64, u64)>,
        board: &Board,
        chip: Chip,
    ) -> Result<(u64, u64)> {
        let mut ram = 0;
        let mut ram_segments = Vec::new();
        for &(addr, file_size, mem_size) in segments {
            // A zero size segment doesn't have anything to add to the result.
            if mem_size == 0 {
                continue;
            }

            // First, count everything in the ram, including the stack reserved addresses.
            if let Some(window_end) = Self::window_end(addr, board.ram) {
                // Ensure that the whole segment, up to it's size is inside the same window that
                // it starts in. An error here means that the map is no longer accurately
                // maintaining the windows.
                if addr + mem_size > window_end {
                    bail!(
                        "segment ({addr:#010x}) goes {mem_size} bytes ({:#010x}) past the RAM window",
                        addr + mem_size
                    );
                }

                ram += mem_size;
                ram_segments.push((addr, mem_size));

                continue;
            }

            // Past here the segment is outside every RAM window, so it has to be flash otherwise
            // it's an error.
            if !u32::try_from(addr).is_ok_and(|a| chip.addr_is_flash(a)) {
                bail!("the segment ({addr:#010x}) is not in the RAM window and not flash");
            }

            // Past here it is at a flash address, but if it is completely empty, that's also
            // an error as it's a contradiction.
            if file_size == 0 {
                bail!("segment at {addr:#010x} is a flash address but has zero size");
            }
        }

        // Then, subtract the stack reserved in order to determine the reserved bytes vs the
        // actual RAM bytes.
        let (stack_end, stack_start) = stack_span;
        if stack_start < stack_end {
            bail!("the stack reservation start is less than the end");
        }
        let stack = stack_start - stack_end;

        // This subtraction is only meaningful if the stack reservation is actually inside the
        // windows that were counted. Just in case it was not, error here to avoid an incorrect
        // value.
        if !Self::covered(&ram_segments, stack_end, stack_start) {
            bail!(
                "stack reservation ({stack_end:#010x}..{stack_start:#010x}) is not inside any RAM window"
            );
        }

        let rwdata_dummy = Self::dummy_size(".rwdata_dummy", rwdata_dummy, &ram_segments)?;
        let rtc_fast_dummy = Self::dummy_size(".rtc_fast.dummy", rtc_fast_dummy, &ram_segments)?;

        // Remove both the stack and the dummy size sections from the RAM.
        let ram = ram
            .checked_sub(stack + rwdata_dummy + rtc_fast_dummy)
            .with_context(|| "subtracting bytes from RAM windows overflows".to_string())?;

        Ok((ram, stack))
    }

    /// The total size of the `name` section span, which is counted twice and must be removed.
    /// On Xtensa, these represent the same physical address in two windows, so they are
    /// removed here to avoid double counting.
    ///
    /// See: <https://github.com/esp-rs/esp-hal/blob/esp-hal-v1.1.1/esp-hal/ld/esp32s3/esp32s3.x>
    pub fn dummy_size(
        name: &str,
        section: Option<(u64, u64)>,
        ram_segments: &[(u64, u64)],
    ) -> Result<u64> {
        // A missing or empty dummy section means nothing to avoid double counting.
        let Some((addr, size)) = section else {
            return Ok(0);
        };
        if size == 0 {
            return Ok(0);
        }

        // This subtraction is only meaningful if the dummy section is actually inside the
        // windows that were counted, just like the stack.
        if !Self::covered(ram_segments, addr, addr + size) {
            bail!(
                "`{name}` ({addr:#010x}..{:#010x}) is not inside any RAM window.",
                addr + size
            );
        }

        Ok(size)
    }

    /// The span from `_stack_start` to `_stack_end` which determines stack reserved addresses.
    ///
    /// See: <https://github.com/esp-rs/esp-hal/blob/esp-hal-v1.1.1/esp-hal/ld/sections/stack.x>
    pub fn stack_span(file: &File, elf: &Path) -> Result<(u64, u64)> {
        let symbol = |name: &str| {
            file.symbols()
                .find(|symbol| symbol.name() == Ok(name))
                .map(|symbol| symbol.address())
                .with_context(|| format!("{}: no `{name}` symbol", elf.display()))
        };

        Ok((symbol("_stack_end")?, symbol("_stack_start")?))
    }

    /// The address and size of the `name` section, if it is present.
    fn section_span(file: &File, name: &str) -> Option<(u64, u64)> {
        let section = file.section_by_name(name)?;
        Some((section.address(), section.size()))
    }

    /// True if the `start` and `end` sections are inside one of `segments`.
    fn covered(segments: &[(u64, u64)], start: u64, end: u64) -> bool {
        segments
            .iter()
            .any(|&(addr, size)| addr <= start && end <= addr + size)
    }

    /// The end of the `start` and `end` window containing `addr`, if any.
    fn window_end(addr: u64, windows: &[(u64, u64)]) -> Option<u64> {
        windows
            .iter()
            .find(|&&(lo, hi)| (lo..hi).contains(&addr))
            .map(|&(_, hi)| hi)
    }
}

/// The stack addresses the stack probe paints.
///
/// See: <https://github.com/esp-rs/esp-hal/blob/esp-hal-v1.1.1/esp-hal/ld/sections/stack.x>
#[derive(Debug, Clone, Copy)]
pub struct StackRegion {
    /// The start of the reservation, `_stack_start`.
    pub start: u64,
    /// The bottom of the reservation, `_stack_end`.
    pub end: u64,
    /// The lowest address the probe can reach, one word above the
    /// `__stack_chk_guard`.
    pub floor: u64,
}

impl StackRegion {
    /// Reads the stack out of the `elf` symbols.
    pub fn new(elf: &Path) -> Result<StackRegion> {
        let data = read(elf)?;
        let file = File::parse(&*data).with_context(|| format!("parsing {}", elf.display()))?;

        let (end, top) = Footprint::stack_span(&file, elf)?;
        let guard = file
            .symbols()
            .find(|symbol| symbol.name() == Ok("__stack_chk_guard"))
            .map(|symbol| symbol.address())
            .with_context(|| format!("{}: no `__stack_chk_guard` symbol", elf.display()))?;
        let floor = guard + 4;

        if guard < end || floor >= top {
            bail!("`__stack_chk_guard` ({guard:#010x}) is outside the stack reservation");
        }

        Ok(StackRegion {
            start: top,
            end,
            floor,
        })
    }

    /// The painted and scanned span in whole words.
    pub fn words(&self) -> usize {
        usize::try_from((self.start - self.floor) / 4).expect("stack reservation fits in memory")
    }
}

fn read(elf: &Path) -> Result<Vec<u8>> {
    std::fs::read(elf).with_context(|| format!("reading {}", elf.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::find;

    fn board_and_chip(name: &str) -> (&'static Board, Chip) {
        let board = find(name).unwrap();
        (board, Chip::from_str(board.soc).unwrap())
    }

    #[test]
    fn windows_are_open() {
        let board = find("esp32c6-devkitc").unwrap();
        let chip = Chip::from_str(board.soc).unwrap();

        assert!(Footprint::window_end(0x4081_3800, board.ram).is_some());
        assert!(chip.addr_is_flash(0x4202_0020));
        assert!(Footprint::window_end(0x4202_0020, board.ram).is_none());
        assert!(Footprint::window_end(0x4080_0000, board.ram).is_some());
        assert!(Footprint::window_end(0x4087_E610, board.ram).is_none());
        assert!(Footprint::window_end(0, board.ram).is_none() && !chip.addr_is_flash(0));
    }

    #[test]
    fn segment_should_be_starting_window() {
        let board = find("esp32-s2-saola").unwrap();
        let (addr, mem_size) = (0x3FFF_0000, 0x3_1000);
        let end = Footprint::window_end(addr, board.ram).unwrap();

        assert!(Footprint::window_end(addr + mem_size - 1, board.ram).is_some());
        assert!(addr + mem_size > end);
        assert_eq!(
            Footprint::window_end(0x3FFB_0000, board.ram),
            Some(0x4000_0000)
        );
        assert!(Footprint::window_end(0x4000_0000, board.ram).is_none());
    }

    #[test]
    fn covered_requires_segment() {
        let segments = [(0x100, 0x100), (0x200, 0x100)];

        assert!(Footprint::covered(&segments, 0x100, 0x200));
        assert!(Footprint::covered(&segments, 0x210, 0x280));
        assert!(!Footprint::covered(&segments, 0x180, 0x220));
        assert!(!Footprint::covered(&[], 0x100, 0x200));
    }

    #[test]
    fn measures_ram_flash_and_stack() {
        let (board, chip) = board_and_chip("esp32c6-devkitc");
        let segments = [
            (0x4080_0000, 0x100, 0x300),
            (0x5000_0000, 0x10, 0x10),
            (0x4202_0000, 0x40, 0x40),
            (0x1000, 0, 0),
        ];
        let stack_span = (0x4080_0200, 0x4080_0300);
        let (ram, stack) =
            Footprint::memory_of(&segments, stack_span, None, None, board, chip).unwrap();
        assert_eq!(stack, 0x100);
        assert_eq!(ram, 0x300 + 0x10 - 0x100);
    }

    #[test]
    fn segment_outside_windows_error() {
        let (board, chip) = board_and_chip("esp32-s2-saola");
        let segments = [(0x3FFF_0000, 0, 0x3_1000)];
        assert!(Footprint::memory_of(&segments, (0, 0), None, None, board, chip).is_err());

        let (board, chip) = board_and_chip("esp32c6-devkitc");
        let segments = [(0x1000, 0x10, 0x10)];
        assert!(Footprint::memory_of(&segments, (0, 0), None, None, board, chip).is_err());

        let (board, chip) = board_and_chip("esp32c6-devkitc");
        let segments = [(0x4202_0020, 0, 0x10)];
        assert!(Footprint::memory_of(&segments, (0, 0), None, None, board, chip).is_err());

        let (board, chip) = board_and_chip("esp32c6-devkitc");
        let segments = [(0x4080_0000, 0x100, 0x100)];
        let stack_span = (0x5000_0000, 0x5000_1000);
        assert!(Footprint::memory_of(&segments, stack_span, None, None, board, chip).is_err());

        let (board, chip) = board_and_chip("esp32c6-devkitc");
        let segments = [(0x4080_0000, 0x100, 0x100)];
        let stack_span = (0x5000_0000, 0x5000_1000);
        assert!(Footprint::memory_of(&segments, stack_span, None, None, board, chip).is_err());

        let (board, chip) = board_and_chip("esp32c6-devkitc");
        let segments = [(0x4080_0000, 0x100, 0x100)];
        let stack_span = (0x4080_0300, 0x4080_0200);
        assert!(Footprint::memory_of(&segments, stack_span, None, None, board, chip).is_err());
    }

    #[test]
    fn dummy_sections_are_subtracted() {
        let (board, chip) = board_and_chip("waveshare-esp32-s3-touch-lcd-43");
        let segments = [(0x3FC8_8000, 0x400, 0x400), (0x4037_0000, 0x100, 0x100)];

        let (ram, stack) = Footprint::memory_of(
            &segments,
            (0x3FC8_8200, 0x3FC8_8300),
            Some((0x3FC8_8000, 0x100)),
            Some((0x3FC8_8100, 0x40)),
            board,
            chip,
        )
        .unwrap();

        assert_eq!(stack, 0x100);
        assert_eq!(ram, 0x400 + 0x100 - 0x100 - 0x100 - 0x40);
    }

    #[test]
    fn dummy_section_outside_window_error() {
        let counted = [(0x3FC8_8000, 0x400)];
        assert!(
            Footprint::dummy_size(".rwdata_dummy", Some((0x4037_0000, 0x100)), &counted).is_err()
        );
        assert_eq!(
            Footprint::dummy_size(".rtc_fast.dummy", Some((0, 0)), &[]).unwrap(),
            0
        );
        assert_eq!(
            Footprint::dummy_size(".rotext_dummy", None, &[]).unwrap(),
            0
        );
    }
}
