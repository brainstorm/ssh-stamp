// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Host tests for the BSP codegen.
//!
//! These are the tests the old build scripts could not have: build-script
//! code only runs during a build, so the only way to check it was to build
//! firmware and read the generated file. Now the logic is a library, the
//! emitters can be driven directly against the fixture boards in
//! `tests/fixtures/boards/`.

use std::path::Path;

use board_codegen::emit::{Column, MacroShell, PinField::Pin, Pins};
use board_codegen::{Board, Target, emit, load_boards_from, or_dash};
use serde::Deserialize;

/// Stand-in for a real target's extra TOML tables. Deliberately reads both
/// an extra key *inside* `[pins]` and a whole extra table, which is the
/// shape the ESP32 BSP needs.
#[derive(Debug, Deserialize)]
struct TestExt {
    #[serde(default)]
    pins: ExtPins,
    widget: Option<Widget>,
}

#[derive(Debug, Default, Deserialize)]
struct ExtPins {
    widget_pin: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct Widget {
    enabled: bool,
}

/// Concrete-singleton spelling, as embassy-rp needs.
const CONCRETE: Target = Target {
    crate_name: "ssh-stamp-test-boards",
    pin_token: |n| format!("$peripherals.PIN_{n}"),
};

/// `AnyPin`-erasing spelling, as esp-hal uses.
const ERASED: Target = Target {
    crate_name: "ssh-stamp-test-boards",
    pin_token: |n| format!("$peripherals.GPIO{n}.into()"),
};

fn boards() -> Vec<Board<TestExt>> {
    load_boards_from(Path::new("tests/fixtures/boards")).expect("fixtures load")
}

#[test]
fn loads_boards_sorted_with_names_derived_from_the_filename() {
    let boards = boards();
    let names: Vec<&str> = boards.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["alpha-devkit", "beta-plain"], "sorted by filename");

    let alpha = &boards[0];
    assert_eq!(alpha.struct_name, "AlphaDevkit");
    assert_eq!(alpha.feature, "board-alpha-devkit");
    assert_eq!(alpha.url.as_deref(), Some("https://example.invalid/alpha"));
    assert_eq!((alpha.uart_rx, alpha.uart_tx), (10, 11));

    assert_eq!(boards[1].url, None, "a board may omit its url");
}

#[test]
fn extension_reads_extra_tables_and_extra_keys_inside_pins() {
    let boards = boards();

    // Whole extra table.
    assert!(boards[0].ext.widget.as_ref().is_some_and(|w| w.enabled));
    // Extra key inside [pins], alongside the uart keys the common schema
    // owns. This is why the loader deserializes the whole document twice.
    assert_eq!(boards[0].ext.pins.widget_pin, Some(7));

    assert!(boards[1].ext.widget.is_none());
    assert_eq!(boards[1].ext.pins.widget_pin, None);
}

#[test]
fn emits_the_board_trait_above_the_impls_that_need_it() {
    let mut out = String::new();
    emit::board_trait(&mut out);
    emit::structs(&mut out, &boards()).unwrap();

    let trait_at = out.find("pub trait Board").expect("trait emitted");
    let impl_at = out
        .find("impl Board for AlphaDevkit")
        .expect("impl emitted");
    assert!(
        trait_at < impl_at,
        "the trait must precede the impls; they name it unqualified"
    );
    assert!(out.contains("pub struct AlphaDevkit;"));
    assert!(out.contains(r#"const NAME: &str = "beta-plain";"#));
}

#[test]
fn catalog_renders_extra_columns_and_dashes_missing_values() {
    let extra = [Column {
        header: "Widget",
        cell: |b: &Board<TestExt>| or_dash(b.ext.pins.widget_pin),
    }];

    let mut out = String::new();
    emit::catalog(&mut out, &boards(), &extra).unwrap();

    assert!(out.contains("| Board feature | UART RX | UART TX | Widget | URL |"));
    assert!(out.contains("|---|---|---|---|---|"));
    assert!(
        out.contains("| `board-alpha-devkit` | 10 | 11 | 7 | <https://example.invalid/alpha> |")
    );
    // No url and no widget: both fall back to an em dash.
    assert!(out.contains("| `board-beta-plain` | 2 | 3 | — | — |"));
    assert!(out.contains("pub mod board_catalog {}"));
}

#[test]
fn pin_spelling_comes_from_the_target() {
    let boards = boards();

    let mut concrete = String::new();
    emit::uart_pins(&mut concrete, &CONCRETE, &boards);
    assert!(concrete.contains("$peripherals.PIN_10,"));
    assert!(!concrete.contains(".into()"));

    let mut erased = String::new();
    emit::uart_pins(&mut erased, &ERASED, &boards);
    assert!(erased.contains("$peripherals.GPIO10.into(),"));

    // Both spellings still emit the raw numbers the config persists.
    for out in [&concrete, &erased] {
        assert!(out.contains("10u8,") && out.contains("11u8,"));
    }
}

#[test]
fn pin_macro_emits_a_compile_error_for_boards_missing_the_section() {
    let shell = MacroShell {
        name: "take_widget_pins",
        doc: "/// Doc.\n",
    };

    let mut out = String::new();
    emit::pin_macro(&mut out, &CONCRETE, &shell, &boards(), |b| -> Pins {
        b.ext
            .pins
            .widget_pin
            .map(|p| vec![Pin(p)])
            .ok_or_else(|| format!("Board `{}` has no widget.", b.feature))
    });

    assert!(out.contains("macro_rules! take_widget_pins"));
    assert!(out.contains(r#"#[cfg(feature = "board-alpha-devkit")]"#));
    assert!(out.contains("$peripherals.PIN_7,"));
    // The board without the section gets a readable compile_error, not a
    // silently missing branch.
    assert!(out.contains(r#"compile_error!("Board `board-beta-plain` has no widget.");"#));
}

#[test]
fn macro_shell_omits_boards_whose_branch_is_none() {
    let shell = MacroShell {
        name: "setup_widget",
        doc: "/// Doc.\n",
    };

    let mut out = String::new();
    emit::macro_shell(&mut out, &CONCRETE, &shell, &boards(), |b| {
        b.ext
            .widget
            .as_ref()
            .map(|_| "        { widget(); }\n".to_string())
    });

    assert!(out.contains(r#"#[cfg(feature = "board-alpha-devkit")]"#));
    assert!(
        !out.contains(r#"#[cfg(feature = "board-beta-plain")]"#),
        "a None branch leaves the board out entirely, making the macro a no-op there"
    );
}

#[test]
fn every_macro_falls_back_to_a_message_naming_the_crate() {
    let boards = boards();
    let mut out = String::new();
    emit::uart_pins(&mut out, &CONCRETE, &boards);

    assert!(out.contains(
        r#"#[cfg(not(any(feature = "board-alpha-devkit", feature = "board-beta-plain")))]"#
    ));
    assert!(out.contains("See ssh-stamp-test-boards crate for available boards."));
}

#[test]
fn select_board_emits_one_alias_per_board() {
    let mut out = String::new();
    emit::select_board(&mut out, &boards());

    assert!(out.contains("macro_rules! select_board"));
    assert!(out.contains("type B = $crate::AlphaDevkit;"));
    assert!(out.contains("type B = $crate::BetaPlain;"));
    assert!(out.contains(r#"compile_error!("No board feature selected.");"#));
}

#[test]
fn generated_file_parses_as_rust() {
    // The whole point of the codegen is to emit valid Rust. Catching a
    // malformed template here beats discovering it in a firmware build.
    let boards = boards();
    let mut out = String::new();
    emit::header(&mut out);
    emit::board_trait(&mut out);
    emit::structs(&mut out, &boards).unwrap();
    emit::catalog(&mut out, &boards, &[]).unwrap();
    emit::uart_pins(&mut out, &CONCRETE, &boards);
    emit::select_board(&mut out, &boards);

    // Balanced delimiters are a cheap proxy for "this will tokenize".
    let opens = out.matches('{').count();
    let closes = out.matches('}').count();
    assert_eq!(opens, closes, "unbalanced braces in generated code:\n{out}");
    assert!(out.starts_with("// Auto-generated by build.rs"));
}
