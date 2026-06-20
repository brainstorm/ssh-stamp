// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Board Support Package for ssh-stamp-esp32.
//!
//! Each board feature selects a specific PCB and provides both the logical
//! pin numbers (via [`Board::UART_RX`] / [`Board::UART_TX`]) and the
//! hardware GPIO extraction (via [`take_uart_pins!`]).
//!
//! The [`Board`] trait is the single source of truth for pin numbers. The
//! [`take_uart_pins!`] macro verifies at compile time that its GPIO field
//! access matches the trait's consts, so the two can never drift: changing
//! a pin in the trait without updating the macro (or vice versa) is a
//! compile error.
//!
//! # Available boards
//!
//! | Board feature            | IC        | UART RX | UART TX | Board                                                                              |
//! |--------------------------|-----------|---------|---------|------------------------------------------------------------------------------------|
//! | `board-esp32c6-devkitc`  | ESP32-C6  | 10      | 11      | [ESP32-C6-DevKitC-1](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/index.html) |
//! | `board-esp32c6-generic`  | ESP32-C6  | 10      | 11      | Generic ESP32-C6 board                                                              |
//! | `board-esp32-s2-saola`   | ESP32-S2  | 10      | 11      | [ESP32-S2-Saola-1](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s2/esp32-s2-saola-1/index.html) |

#![no_std]

mod esp32_s2_saola;
mod esp32c6_devkitc;
mod esp32c6_generic;

pub use esp32_s2_saola::Esp32s2Saola;
pub use esp32c6_devkitc::Esp32c6Devkitc;
pub use esp32c6_generic::Esp32c6Generic;

/// Board-level pin assignment.
///
/// Each supported board implements this trait to provide GPIO pin numbers
/// for the UART bridge. These consts are the single source of truth for
/// pin numbers — the [`take_uart_pins!`] macro cross-checks them at
/// compile time against its GPIO field access.
///
/// To add a new board:
/// 1. Create a module in `src/` with a unit struct implementing `Board`.
/// 2. Add a `board-<name>` feature in `Cargo.toml` (implies the IC feature on `esp-hal`).
/// 3. Add the GPIO mapping branch to the `take_uart_pins!` macro in this file,
///    with a `const { assert!(...) }` matching the trait consts.
/// 4. Add the feature gate + `use` in the binary's `cfg_if!` block.
pub trait Board {
    /// Human-readable board name (e.g. `"esp32c6-devkitc"`).
    const NAME: &'static str;
    /// GPIO number for UART RX.
    const UART_RX: u8;
    /// GPIO number for UART TX.
    const UART_TX: u8;
}

/// Extract UART GPIO pins from `peripherals` for the active board.
///
/// Pass the active board type so the macro can verify at compile time that
/// its GPIO field access matches [`Board::UART_RX`] / [`Board::UART_TX`].
/// Returns `(AnyPin, AnyPin)` — the caller wraps these into the appropriate
/// UART pins struct.
///
/// This macro is the **only** place in the codebase where UART GPIO singletons
/// are accessed. The pin numbers in the macro are checked against the `Board`
/// trait consts via `const { assert!(...) }`, so the `Board` trait remains the
/// canonical declaration and the two can never silently drift.
///
/// # Panics
///
/// Compile-time error if no board feature is selected, or if the `Board`
/// trait consts disagree with the macro's GPIO field access.
#[macro_export]
macro_rules! take_uart_pins {
    ($peripherals:expr, $board:ty) => {{
        #[cfg(feature = "board-esp32c6-devkitc")]
        {
            const { assert!(<$board>::UART_RX == 10) };
            const { assert!(<$board>::UART_TX == 11) };
            ($peripherals.GPIO10.into(), $peripherals.GPIO11.into())
        }
        #[cfg(feature = "board-esp32c6-generic")]
        {
            const { assert!(<$board>::UART_RX == 10) };
            const { assert!(<$board>::UART_TX == 11) };
            ($peripherals.GPIO10.into(), $peripherals.GPIO11.into())
        }
        #[cfg(feature = "board-esp32-s2-saola")]
        {
            const { assert!(<$board>::UART_RX == 10) };
            const { assert!(<$board>::UART_TX == 11) };
            ($peripherals.GPIO10.into(), $peripherals.GPIO11.into())
        }
        #[cfg(not(any(
            feature = "board-esp32c6-devkitc",
            feature = "board-esp32c6-generic",
            feature = "board-esp32-s2-saola",
        )))]
        {
            compile_error!("No board feature selected.");
        }
    }};
}
