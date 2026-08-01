// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Board Support Package for ssh-stamp-rp2350.
//!
//! Counterpart of `ssh-stamp-esp32-boards`, with the same contract: board
//! definitions live in `boards/*.toml` (one file per board, containing pin
//! mappings and an optional documentation URL). The `build.rs` reads those
//! TOML files and generates all Rust code — board structs, the
//! [`take_uart_pins!`] macro, the [`take_ethernet_pins!`] macro and the
//! [`select_board!`] macro — into `OUT_DIR/boards_gen.rs`, included here.
//!
//! The TOML files are the single source of truth for pin numbers. No human
//! writes or edits the generated Rust code.
//!
//! # Adding a board
//!
//! 1. Create `boards/{board-name}.toml` with a `[pins]` section (`uart_rx`,
//!    `uart_tx`), an optional `[ethernet]` section, and an optional `url`.
//! 2. Add `board-{name} = []` to `[features]` in `Cargo.toml`.
//!
//! No `.rs` file, no macro editing, no binary changes. The `build.rs`
//! validates that selected features have matching TOML files.
//!
//! # Difference from the ESP32 BSP
//!
//! The pin macros hand back embassy-rp's *concrete* `PIN_n` singletons
//! rather than erasing them to `AnyPin`. embassy-rp constrains UART pins
//! with per-instance `TxPin`/`RxPin` traits that `AnyPin` does not
//! implement, so erasing would make the pins unusable.
//!
//! # Available boards
//!
//! See the [`board_catalog`] module for the generated table.

#![no_std]

/// Board identification trait.
///
/// Each board struct generated from `boards/*.toml` implements this trait.
/// The `NAME` const is the board's filename (without `.toml`), used for
/// boot-time logging.
pub trait Board {
    /// Human-readable board name (e.g. `"w6300-evb-pico2"`).
    const NAME: &'static str;
}

include!(concat!(env!("OUT_DIR"), "/boards_gen.rs"));
