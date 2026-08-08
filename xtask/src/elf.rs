// SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reads the linked firmware ELF directly with the [`object`] crate.
//!
//! This replaces shelling out to the cargo-binutils shims (`rust-size -A -x`,
//! `rust-readobj --stack-sizes`) and parsing their text output. `object` reads
//! any target's ELF regardless of host, so it also drops `llvm-tools` +
//! `cargo-binutils` from the prerequisites of the size gate — one less thing for
//! CI to install and for a contributor to get wrong.

use anyhow::{Context, Result, bail};
use object::{Object, ObjectSection, ObjectSymbol, SymbolKind};
use std::collections::HashMap;
use std::path::Path;

use crate::results::{Section, StackFrame};

/// Every named section in `elf`, with its size in bytes.
pub fn sections(elf: &Path) -> Result<Vec<Section>> {
    let data = read(elf)?;
    let file = object::File::parse(&*data).with_context(|| format!("parsing {}", elf.display()))?;
    Ok(file
        .sections()
        .filter_map(|s| {
            let name = s.name().ok()?;
            // Skip the null section and anything unnamed; keep the ESP-specific
            // ones (`.rwtext`, `.dram0.*`, …) so a reviewer can audit the
            // headline two-section flash/RAM figures.
            (!name.is_empty()).then(|| Section {
                name: name.to_string(),
                size_b: s.size(),
            })
        })
        .collect())
}

/// Sums the sizes of the named sections. Missing sections contribute 0.
pub fn sum_sections(sections: &[Section], names: &[&str]) -> u64 {
    sections
        .iter()
        .filter(|s| names.contains(&s.name.as_str()))
        .map(|s| s.size_b)
        .sum()
}

/// Per-function static stack frames from the `.stack_sizes` section, largest
/// first.
///
/// The section is a sequence of `(function address, ULEB128 frame size)` records
/// emitted by `-Z emit-stack-sizes`; addresses are resolved back to symbol names
/// through the ELF symbol table. Returns an empty vector when the section is
/// absent (fat LTO strips it, and it only exists on a nightly build) — the
/// caller warns rather than failing, since this is a diagnostic, not a gate.
pub fn stack_frames(elf: &Path) -> Result<Vec<StackFrame>> {
    let data = read(elf)?;
    let file = object::File::parse(&*data).with_context(|| format!("parsing {}", elf.display()))?;

    let Some(section) = file.section_by_name(".stack_sizes") else {
        return Ok(Vec::new());
    };
    let raw = section
        .data()
        .map_err(|e| anyhow::anyhow!("reading .stack_sizes of {}: {e}", elf.display()))?;

    // address -> symbol name, for the function symbols the records point at.
    let names: HashMap<u64, &str> = file
        .symbols()
        .filter(|s| s.kind() == SymbolKind::Text)
        .filter_map(|s| s.name().ok().map(|n| (s.address(), n)))
        .collect();

    let addr_bytes = if file.is_64() { 8 } else { 4 };
    let little_endian = file.is_little_endian();
    let mut frames = Vec::new();
    let mut rest = raw;

    while !rest.is_empty() {
        if rest.len() < addr_bytes {
            bail!(
                "truncated .stack_sizes in {}: {} trailing byte(s)",
                elf.display(),
                rest.len()
            );
        }
        let (addr_raw, tail) = rest.split_at(addr_bytes);
        let address = read_addr(addr_raw, little_endian);
        let (size_b, tail) = read_uleb128(tail)
            .with_context(|| format!("decoding .stack_sizes frame size in {}", elf.display()))?;
        rest = tail;

        frames.push(StackFrame {
            // Demangled, because the table exists to name the function worth
            // shrinking and a mangled symbol names it only to a demangler. An
            // unresolved address still carries a usable frame size.
            //
            // `{:#}` is the alternate form, which drops the crate disambiguator
            // hashes: they change with the build, so keeping them would make the
            // same function look like a different one from run to run — the one
            // thing a table meant for tracking regressions must not do.
            function: names.get(&address).map_or_else(
                || format!("<unnamed @ {address:#x}>"),
                |n| format!("{:#}", rustc_demangle::demangle(n)),
            ),
            size_b,
        });
    }

    frames.sort_by_key(|f| std::cmp::Reverse(f.size_b));
    Ok(frames)
}

fn read(elf: &Path) -> Result<Vec<u8>> {
    std::fs::read(elf).with_context(|| {
        format!(
            "reading {} — build it first, or drop --no-build",
            elf.display()
        )
    })
}

/// Decodes a 4- or 8-byte address in the ELF's endianness.
fn read_addr(bytes: &[u8], little_endian: bool) -> u64 {
    bytes.iter().enumerate().fold(0u64, |acc, (i, &b)| {
        let shift = if little_endian {
            8 * i
        } else {
            8 * (bytes.len() - 1 - i)
        };
        acc | (u64::from(b) << shift)
    })
}

/// Decodes one ULEB128 integer, returning it and the remaining bytes.
fn read_uleb128(bytes: &[u8]) -> Result<(u64, &[u8])> {
    let mut value = 0u64;
    let mut shift = 0u32;
    for (i, &byte) in bytes.iter().enumerate() {
        if shift >= 64 {
            bail!("ULEB128 value wider than 64 bits");
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, &bytes[i + 1..]));
        }
        shift += 7;
    }
    bail!("unterminated ULEB128 value")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_single_byte_uleb128() {
        assert_eq!(read_uleb128(&[0x10, 0xAA]).unwrap(), (0x10, &[0xAAu8][..]));
        assert_eq!(read_uleb128(&[0x7f]).unwrap().0, 127);
    }

    #[test]
    fn reads_multi_byte_uleb128() {
        // 624485 == 0xE5 0x8E 0x26, the canonical LEB128 example.
        assert_eq!(read_uleb128(&[0xE5, 0x8E, 0x26]).unwrap().0, 624_485);
        // 128 needs a continuation byte.
        assert_eq!(read_uleb128(&[0x80, 0x01]).unwrap().0, 128);
    }

    #[test]
    fn rejects_unterminated_uleb128() {
        assert!(read_uleb128(&[0x80, 0x80]).is_err());
        assert!(read_uleb128(&[]).is_err());
    }

    #[test]
    fn reads_addresses_in_both_endiannesses() {
        assert_eq!(read_addr(&[0x20, 0x00, 0x00, 0x42], true), 0x4200_0020);
        assert_eq!(read_addr(&[0x42, 0x00, 0x00, 0x20], false), 0x4200_0020);
    }

    #[test]
    fn sums_only_the_named_sections() {
        let s = vec![
            Section {
                name: ".text".into(),
                size_b: 100,
            },
            Section {
                name: ".rodata".into(),
                size_b: 20,
            },
            Section {
                name: ".bss".into(),
                size_b: 7,
            },
        ];
        assert_eq!(sum_sections(&s, &[".text", ".rodata"]), 120);
        // A section that isn't present contributes nothing rather than erroring.
        assert_eq!(sum_sections(&s, &[".data", ".bss"]), 7);
    }
}
