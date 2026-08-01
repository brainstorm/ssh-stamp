// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Turning a board file stem into the names the generated code uses.

/// Convert a file stem to a Rust type name: `esp32c6-devkitc` ->
/// `Esp32c6Devkitc`, `w6300-evb-pico2` -> `W6300EvbPico2`.
#[must_use]
pub fn to_pascal_case(s: &str) -> String {
    s.split('-')
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Convert a file stem to its cargo feature: `esp32c6-devkitc` ->
/// `board-esp32c6-devkitc`.
#[must_use]
pub fn feature_name(stem: &str) -> String {
    format!("board-{stem}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case_matches_the_generated_struct_names() {
        assert_eq!(to_pascal_case("esp32c6-devkitc"), "Esp32c6Devkitc");
        assert_eq!(to_pascal_case("esp32-s2-saola"), "Esp32S2Saola");
        assert_eq!(to_pascal_case("w6300-evb-pico2"), "W6300EvbPico2");
        assert_eq!(
            to_pascal_case("waveshare-esp32-s3-touch-lcd-43"),
            "WaveshareEsp32S3TouchLcd43"
        );
    }

    #[test]
    fn feature_names_are_prefixed() {
        assert_eq!(feature_name("esp32c6-devkitc"), "board-esp32c6-devkitc");
    }
}
