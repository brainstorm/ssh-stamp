// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `select_board!` macro.

use crate::load::Board;
use crate::render::{feature_list, render};

const TMPL: &str = r#"/// Select the active board type `B` at compile time.
///
/// Emits a `type B = ...` alias for the active board's struct. The caller can
/// then use `B::NAME` for logging. Zero per-board lines in the binary.
///
/// # Panics
///
/// Compile-time error if no board feature is selected.
#[macro_export]
macro_rules! select_board {
    () => {
{arms}        #[cfg(not(any({features})))]
        compile_error!("No board feature selected.");
    };
}
"#;

const ARM: &str = r#"        #[cfg(feature = "{feature}")]
        type B = $crate::{struct_name};
"#;

/// Emit `select_board!`: one `#[cfg]`-guarded `type B` alias per board, so
/// the binary has zero per-board lines.
pub fn select_board<E>(out: &mut String, boards: &[Board<E>]) {
    let mut arms = String::new();
    for b in boards {
        arms.push_str(&render(
            ARM,
            &[("feature", &b.feature), ("struct_name", &b.struct_name)],
        ));
    }

    out.push_str(&render(
        TMPL,
        &[("arms", &arms), ("features", &feature_list(boards))],
    ));
}
