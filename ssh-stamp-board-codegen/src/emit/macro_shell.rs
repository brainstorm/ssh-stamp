// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The skeleton shared by every generated `macro_rules!`.
//!
//! Every macro a BSP emits has the same shape: a rustdoc block, a
//! `#[macro_export] macro_rules! <name> { ($peripherals:expr) => {{ … }} }`,
//! one `#[cfg(feature = …)]` branch per board, and a `not(any(…))` fallback
//! that fails the build with a readable message when no board is selected.
//! Only the branch bodies differ, so those are all a caller supplies.

use crate::Target;
use crate::load::Board;
use crate::render::{feature_list, render};

/// Identity of a generated macro.
pub struct MacroShell<'a> {
    /// Macro name, e.g. `take_uart_pins`.
    pub name: &'a str,
    /// Rustdoc block: `///`-prefixed lines, ending with a newline.
    pub doc: &'a str,
}

const SHELL: &str = r#"{doc}#[macro_export]
macro_rules! {name} {
    ($peripherals:expr) => {{
{branches}        #[cfg(not(any({features})))]
        {{
            compile_error!("No board feature selected. Pass --features board-<name>. See {crate_name} crate for available boards.");
        }}
    }};
}
"#;

const CFG_LINE: &str = "        #[cfg(feature = \"{feature}\")]\n";

/// Emit a macro whose per-board branch bodies come from `branch`.
///
/// `branch` returns the text following the `#[cfg(feature = …)]` line
/// (indentation and braces included), or `None` to leave that board out of
/// the macro entirely — which is how a macro becomes a no-op for boards
/// that do not need it.
pub fn macro_shell<E>(
    out: &mut String,
    target: &Target,
    shell: &MacroShell<'_>,
    boards: &[Board<E>],
    branch: impl Fn(&Board<E>) -> Option<String>,
) {
    let mut branches = String::new();
    for board in boards {
        if let Some(body) = branch(board) {
            branches.push_str(&render(CFG_LINE, &[("feature", &board.feature)]));
            branches.push_str(&body);
        }
    }

    out.push_str(&render(
        SHELL,
        &[
            ("doc", shell.doc),
            ("name", shell.name),
            ("branches", &branches),
            ("features", &feature_list(boards)),
            ("crate_name", target.crate_name),
        ],
    ));
    out.push('\n');
}
