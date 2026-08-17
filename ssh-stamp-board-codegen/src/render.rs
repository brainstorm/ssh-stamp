// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Slot substitution for the code templates.

use std::fmt;

use crate::load::Board;

/// Substitute `{name}` slots in a raw-string template.
///
/// Deliberately *not* `format!`. The templates are Rust source and contain
/// literal braces that must survive untouched: the `{{ … }}` blocks inside
/// `macro_rules!` bodies, and `{e:?}` inside a generated `log::warn!`.
/// `format!` would force escaping every one of them and bury the shape of
/// the emitted code.
#[must_use]
pub fn render(template: &str, slots: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in slots {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// `feature = "board-a", feature = "board-b", …` for a `cfg(not(any(…)))`
/// guard listing every known board.
#[must_use]
pub fn feature_list<E>(boards: &[Board<E>]) -> String {
    boards
        .iter()
        .map(|b| format!("feature = \"{}\"", b.feature))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render an optional value for a catalog cell, using an em dash for `None`.
#[must_use]
pub fn or_dash<T: fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "—".to_string(), |v| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_named_slots() {
        assert_eq!(render("a {x} c", &[("x", "b")]), "a b c");
    }

    #[test]
    fn leaves_rust_braces_alone() {
        // The whole reason `render` exists instead of `format!`: doubled
        // braces in macro bodies and `{e:?}` in generated code must survive.
        let template = "{{ let _ = {name}; warn!(\"{e:?}\"); }}";
        assert_eq!(
            render(template, &[("name", "x")]),
            "{{ let _ = x; warn!(\"{e:?}\"); }}"
        );
    }

    #[test]
    fn unknown_slots_are_left_untouched() {
        assert_eq!(render("{a}{b}", &[("a", "1")]), "1{b}");
    }
}
