// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build script for ssh-stamp-rp2350-boards.
//!
//! Same contract as `ssh-stamp-esp32-boards`: read `boards/*.toml` and
//! generate `OUT_DIR/boards_gen.rs`. Everything that is not RP2350-specific
//! lives in `ssh-stamp-board-codegen`; this file declares only the pin
//! spelling, the `[ethernet]` table and the `take_ethernet_pins!` macro.
//!
//! Adding a board = add a `boards/{name}.toml` file + a `board-{name}`
//! feature in `Cargo.toml`. No `.rs` file, no macro editing.

use board_codegen::emit::{Column, MacroShell, PinField::Pin, Pins};
use board_codegen::{Board, Result, Target, emit, load_boards, validate, write_generated};
use serde::Deserialize;

/// Unlike the ESP32 BSP, the pin macros expand to the *concrete*
/// `peripherals.PIN_n` fields rather than erasing to `AnyPin`: embassy-rp
/// constrains UART pins with per-instance `TxPin`/`RxPin` traits that
/// `AnyPin` does not implement.
const TARGET: Target = Target {
    crate_name: "ssh-stamp-rp2350-boards",
    pin_token: |n| format!("$peripherals.PIN_{n}"),
};

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=boards/");

    let boards: Vec<Board<Rp2350>> = load_boards()?;
    validate::features(&boards)?;

    let mut out = String::new();
    emit::header(&mut out);
    emit::board_trait(&mut out);
    emit::structs(&mut out, &boards)?;
    emit::catalog(&mut out, &boards, &CATALOG_COLUMNS)?;
    emit::uart_pins(&mut out, &TARGET, &boards);
    ethernet_pins(&mut out, &boards);
    emit::select_board(&mut out, &boards);

    write_generated(&out)
}

// --- The RP2350-specific slice of a boards/*.toml -----------------------

#[derive(Debug, Deserialize)]
struct Rp2350 {
    ethernet: Option<Ethernet>,
}

/// Onboard Ethernet controller wiring.
#[derive(Debug, Deserialize)]
struct Ethernet {
    chip: String,
    int: u8,
    cs: u8,
    sck: u8,
    io0: u8,
    io1: u8,
    rst: u8,
}

// --- Catalog ------------------------------------------------------------

const CATALOG_COLUMNS: [Column<Rp2350>; 1] = [Column {
    header: "Ethernet",
    cell: |b| {
        b.ext.ethernet.as_ref().map_or_else(
            || "—".to_string(),
            |e| {
                format!(
                    "{} (int {}, cs {}, sck {}, io0 {}, io1 {}, rst {})",
                    e.chip, e.int, e.cs, e.sck, e.io0, e.io1, e.rst
                )
            },
        )
    },
}];

// --- take_ethernet_pins! ------------------------------------------------

const ETHERNET_PINS: MacroShell = MacroShell {
    name: "take_ethernet_pins",
    doc: r"/// Extract the Ethernet controller's GPIO pins from `peripherals`.
///
/// Returns `(int, cs, sck, io0, io1, rst)`. Only call this macro on boards
/// whose TOML declares an `[ethernet]` section.
///
/// # Panics
///
/// Compile-time error if no board feature is selected, or if the selected
/// board has no Ethernet controller.
",
};

fn ethernet_pins(out: &mut String, boards: &[Board<Rp2350>]) {
    emit::pin_macro(out, &TARGET, &ETHERNET_PINS, boards, |b| -> Pins {
        match &b.ext.ethernet {
            Some(e) => Ok(vec![
                Pin(e.int),
                Pin(e.cs),
                Pin(e.sck),
                Pin(e.io0),
                Pin(e.io1),
                Pin(e.rst),
            ]),
            None => Err(format!(
                "Board `{}` has no [ethernet] section in its boards/*.toml, so it has no Ethernet pins to take.",
                b.feature
            )),
        }
    });
}
