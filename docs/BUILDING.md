<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# Building

Everything is driven by `cargo xtask`, which knows every board, chip,
toolchain and target triple the project supports. Start with:

```
cargo xtask list        # what can I build?
cargo xtask             # usage
```

Tooling is controlled by `rust-toolchain.toml`. On a fresh host you'll typically need the Rust source component and a flasher (we use `espflash` below as an example):

```
rustup toolchain install stable --component rust-src
cargo install espflash --locked
```

Xtensa targets (ESP32/ESP32-S2/S3) do require `espup` in addition to `rustup`:

```
cargo install espup
espup install
source $HOME/export-esp.sh
```

## Building

Pass either a **board** (builds the firmware binary) or a bare **chip**
(builds the library only, for chips with no board definition yet):

```
cargo xtask build esp32c6-devkitc                    # a board
cargo xtask build esp32c3                            # a chip, library only
cargo xtask build waveshare-esp32-s3-touch-lcd-43    # Xtensa, picks the esp toolchain itself
```

Optional features and profiles are flags rather than separate commands, and
anything after `--` goes straight to cargo:

```
cargo xtask build esp32c6-devkitc --features sftp-ota
cargo xtask build waveshare-esp32-s3-touch-lcd-43 --features can-no-ack
cargo xtask build esp32c6-devkitc --profile dev -- --timings
```

You do not need to remember which targets need `+esp`, `-Zbuild-std` or a
special profile: that lives in [`xtask/targets.toml`](../xtask/targets.toml)
and in the `[build]` section of each board definition under
`ssh-stamp-esp32-boards/boards/`.

## Flashing

`run` builds, flashes and opens the serial monitor (via the `runner`
configured per target triple in `.cargo/config.toml`):

```
cargo xtask run esp32c6-devkitc
```

## Everything CI checks

```
cargo xtask clippy      # lints one representative board
cargo xtask fmt         # --check to verify instead of rewrite
cargo xtask doc
cargo xtask test        # host-side crates
cargo xtask ci          # all of the above, for every board and chip
```

## Adding a board

1. Drop a `boards/<name>.toml` into the relevant BSP crate, with the pin map
   and a `[build]` section naming its chip:
   ```toml
   [build]
   chip = "esp32c6"
   # features = ["can"]   # optional: features this board always needs
   ```
2. Add the matching `board-<name>` feature in that platform's `Cargo.toml`.

That is it — `cargo xtask list` picks it up by scanning the boards
directory, so no alias, matrix entry or xtask code has to change.

## Adding a chip or a new vendor

Add a `[chips.<name>]` entry to `xtask/targets.toml` with its target triple
(plus `toolchain`, `build-std` or `profile` if it needs them). A whole new
manufacturer is a `[platforms.<vendor>]` entry pointing at that vendor's
crate and boards directory; nothing in the xtask code is Espressif-specific.
