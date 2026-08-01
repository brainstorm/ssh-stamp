// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Board Support Package codegen, shared by the ssh-stamp BSP build scripts.
//!
//! Every BSP crate (`ssh-stamp-esp32-boards`, `ssh-stamp-rp2350-boards`, …)
//! keeps its board definitions in `boards/*.toml` and turns them into Rust
//! at build time. That machinery — discovering the files, checking them
//! against the selected cargo features, and emitting board structs, the
//! rustdoc catalog and the `macro_rules!` accessors — is identical across
//! targets and lives here. A BSP build script supplies only what actually
//! differs between MCU families:
//!
//! 1. how a pin number is spelled ([`Target::pin_token`]),
//! 2. its extra TOML tables (the `E` in [`Board<E>`]),
//! 3. any extra macros built from them ([`emit::pin_macro`] for tuples of
//!    pins, [`emit::macro_shell`] for anything else).
//!
//! # Shape of a BSP build script
//!
//! ```no_run
//! # use serde::Deserialize;
//! # use board_codegen::{Board, Result, Target, emit, load_boards, validate, write_generated};
//! // Extra tables this target adds to its board files.
//! #[derive(Deserialize)]
//! struct MyTarget {
//!     ethernet: Option<bool>,
//! }
//!
//! const TARGET: Target = Target {
//!     crate_name: "ssh-stamp-mytarget-boards",
//!     pin_token: |n| format!("$peripherals.PIN_{n}"),
//! };
//!
//! fn main() -> Result<()> {
//!     println!("cargo:rerun-if-changed=boards/");
//!     let boards: Vec<Board<MyTarget>> = load_boards()?;
//!     validate::features(&boards)?;
//!
//!     let mut out = String::new();
//!     emit::header(&mut out);
//!     emit::board_trait(&mut out);
//!     emit::structs(&mut out, &boards)?;
//!     emit::catalog(&mut out, &boards, &[])?;
//!     emit::uart_pins(&mut out, &TARGET, &boards);
//!     emit::select_board(&mut out, &boards);
//!     write_generated(&out)
//! }
//! ```
//!
//! This is a host-side crate: build scripts always compile for the host, so
//! unlike the rest of the workspace it is a normal `std` crate.

mod error;
mod load;
mod naming;
mod render;

pub mod emit;
pub mod validate;

use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

pub use error::{BuildError, Result};
pub use load::{Board, load_boards_from};
pub use naming::{feature_name, to_pascal_case};
pub use render::{feature_list, or_dash, render};

/// Everything the codegen needs to know about one MCU family.
pub struct Target<'a> {
    /// This BSP crate's name, quoted in the "no board feature selected"
    /// `compile_error!` so the message points at the right crate.
    pub crate_name: &'a str,

    /// How a pin number is spelled as a peripheral field access.
    ///
    /// esp-hal erases to `AnyPin` (`$peripherals.GPIO10.into()`), while
    /// embassy-rp must keep the concrete singleton
    /// (`$peripherals.PIN_10`) because its per-instance `TxPin`/`RxPin`
    /// traits are not implemented for `AnyPin`.
    pub pin_token: fn(u8) -> String,
}

/// Load `$CARGO_MANIFEST_DIR/boards/*.toml` for the crate being built.
///
/// Reads `CARGO_MANIFEST_DIR` at runtime on purpose: `env!` would bake in
/// *this* crate's directory rather than the BSP crate's.
///
/// # Errors
///
/// Returns an error if `CARGO_MANIFEST_DIR` is unset, or if a board file
/// cannot be read or parsed.
pub fn load_boards<E: DeserializeOwned>() -> Result<Vec<Board<E>>> {
    let manifest_dir =
        std::env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR not set by cargo")?;
    load_boards_from(&PathBuf::from(manifest_dir).join("boards"))
}

/// Write the generated code to `$OUT_DIR/boards_gen.rs`.
///
/// # Errors
///
/// Returns an error if `OUT_DIR` is unset or the file cannot be written.
pub fn write_generated(code: &str) -> Result<()> {
    let out_dir = std::env::var_os("OUT_DIR").ok_or("OUT_DIR not set by cargo")?;
    fs::write(Path::new(&out_dir).join("boards_gen.rs"), code)?;
    Ok(())
}
