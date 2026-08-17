// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build script for ssh-stamp-esp32-boards.
//!
//! Reads `boards/*.toml` and writes `OUT_DIR/boards_gen.rs`. Everything that
//! is not ESP32-specific lives in `ssh-stamp-board-codegen`; this file
//! declares only the three things that make the ESP32 BSP different:
//!
//! 1. pins erase to `AnyPin` via `$peripherals.GPIO{n}.into()`,
//! 2. the extra `[pins] can_tx`/`can_rx` keys and the `[can_mux]` table,
//! 3. the `take_can_pins!` and `setup_can_transceiver!` macros.
//!
//! Adding a board = add a `boards/{name}.toml` file + a `board-{name}`
//! feature in `Cargo.toml`. No `.rs` file, no macro editing.

use board_codegen::emit::{Column, MacroShell, PinField::Pin, Pins};
use board_codegen::{
    Board, Result, Target, emit, load_boards, or_dash, render, validate, write_generated,
};
use serde::Deserialize;

/// esp-hal's GPIO singletons are named fields; `.into()` erases them to
/// `AnyPin`, which the esp-hal UART and TWAI drivers accept.
const TARGET: Target = Target {
    crate_name: "ssh-stamp-esp32-boards",
    pin_token: |n| format!("$peripherals.GPIO{n}.into()"),
};

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=boards/");

    let boards: Vec<Board<Esp32>> = load_boards()?;
    validate::features(&boards)?;

    let mut out = String::new();
    emit::header(&mut out);
    emit::board_trait(&mut out);
    emit::structs(&mut out, &boards)?;
    emit::catalog(&mut out, &boards, &CATALOG_COLUMNS)?;
    emit::uart_pins(&mut out, &TARGET, &boards);
    can_pins(&mut out, &boards);
    setup_can_transceiver(&mut out, &boards);
    emit::select_board(&mut out, &boards);

    write_generated(&out)
}

// --- The ESP32-specific slice of a boards/*.toml ------------------------

/// Deserialized from the *same* document the shared loader reads. Serde
/// ignores the keys it does not name, so this picks up both the extra
/// `[pins]` keys and the whole `[can_mux]` table without any coordination
/// with the common schema.
#[derive(Debug, Deserialize)]
struct Esp32 {
    #[serde(default)]
    pins: CanPins,
    can_mux: Option<CanMux>,
}

#[derive(Debug, Default, Deserialize)]
struct CanPins {
    can_tx: Option<u8>,
    can_rx: Option<u8>,
}

/// Boards that share their CAN pins with other functions behind an
/// I2C-controlled mux (e.g. an IO expander driving an analog switch)
/// declare the routing here: the I2C pins and the `[address, value]`
/// register writes that select the CAN transceiver.
#[derive(Debug, Deserialize)]
struct CanMux {
    i2c_sda: u8,
    i2c_scl: u8,
    writes: Vec<[u8; 2]>,
}

// --- Catalog ------------------------------------------------------------

const CATALOG_COLUMNS: [Column<Esp32>; 2] = [
    Column {
        header: "CAN TX",
        cell: |b| or_dash(b.ext.pins.can_tx),
    },
    Column {
        header: "CAN RX",
        cell: |b| or_dash(b.ext.pins.can_rx),
    },
];

// --- take_can_pins! -----------------------------------------------------

const CAN_PINS: MacroShell = MacroShell {
    name: "take_can_pins",
    doc: r"/// Extract CAN GPIO pins from `peripherals`.
///
/// Returns `(tx_pin, rx_pin)`. The pin numbers come from `boards/*.toml`.
/// Only call this macro when the `can` feature is enabled.
///
/// # Panics
///
/// Compile-time error if no board feature is selected, or if the selected
/// board does not have CAN pins defined.
",
};

fn can_pins(out: &mut String, boards: &[Board<Esp32>]) {
    emit::pin_macro(out, &TARGET, &CAN_PINS, boards, |b| -> Pins {
        match (b.ext.pins.can_tx, b.ext.pins.can_rx) {
            (Some(tx), Some(rx)) => Ok(vec![Pin(tx), Pin(rx)]),
            _ => Err(format!(
                "Board `{}` does not have CAN pins defined. Enable the `can` feature only for boards with CAN support.",
                b.feature
            )),
        }
    });
}

// --- setup_can_transceiver! ---------------------------------------------
//
// The branch bodies are literal esp-hal code, so they live here rather than
// in the shared crate; `emit::macro_shell` supplies the macro skeleton, the
// `#[cfg(feature = ...)]` lines and the no-board-selected fallback.

const SETUP_CAN_TRANSCEIVER: MacroShell = MacroShell {
    name: "setup_can_transceiver",
    doc: r"/// Prepare the board's CAN transceiver routing, if it needs any.
///
/// Boards that share their CAN pins with other functions behind an
/// I2C-controlled mux declare the routing in the `[can_mux]` section of
/// their TOML (I2C pins plus `[address, value]` register writes); this
/// macro performs those writes, consuming the I2C peripheral and mux pins
/// from `peripherals`. Boards without a `[can_mux]` section expand to a
/// no-op. Only call this macro when the `can` feature is enabled.
///
/// # Panics
///
/// Panics if the I2C peripheral cannot be initialised. Compile-time error
/// if no board feature is selected.
",
};

/// The `[can_mux]` branch body. Single braces (not doubled) and the literal
/// `{e:?}` in [`CAN_MUX_WRITE`] are why the codegen substitutes named slots
/// rather than using `format!`.
const CAN_MUX_BRANCH: &str = r#"        {
            let mut i2c = ::esp_hal::i2c::master::I2c::new(
                $peripherals.I2C0,
                ::esp_hal::i2c::master::Config::default(),
            )
            .expect("I2C init error")
            .with_sda($peripherals.GPIO{sda})
            .with_scl($peripherals.GPIO{scl});
{writes}        }
"#;

/// One I2C register write inside a [`CAN_MUX_BRANCH`].
const CAN_MUX_WRITE: &str = r#"            if let Err(e) = i2c.write({addr}u8, &[{value}u8]) {
                ::log::warn!("CAN mux I2C write failed: {e:?}");
            }
"#;

fn setup_can_transceiver(out: &mut String, boards: &[Board<Esp32>]) {
    emit::macro_shell(out, &TARGET, &SETUP_CAN_TRANSCEIVER, boards, |b| {
        // Boards with no `[can_mux]` are skipped entirely: the macro
        // expands to nothing for them.
        let mux = b.ext.can_mux.as_ref()?;

        let mut writes = String::new();
        for [addr, value] in &mux.writes {
            writes.push_str(&render(
                CAN_MUX_WRITE,
                &[
                    ("addr", &format!("{addr:#04x}")),
                    ("value", &format!("{value:#04x}")),
                ],
            ));
        }

        Some(render(
            CAN_MUX_BRANCH,
            &[
                ("sda", &mux.i2c_sda.to_string()),
                ("scl", &mux.i2c_scl.to_string()),
                ("writes", &writes),
            ],
        ))
    });
}
