<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>

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

Name either a **board** (builds the firmware binary) or a library only **chip**
(for chips with no board definition yet), then write the rest of the line as
you would for plain cargo:

```
cargo xtask esp32c6-devkitc build --release                    # a board
cargo xtask esp32c3 build --release                            # a chip, library only
cargo xtask waveshare-esp32-s3-touch-lcd-43 build --release    # Xtensa, picks the esp toolchain itself
```

Because the rest of the line is cargo's, every cargo flag works unchanged:

```
cargo xtask esp32c6-devkitc build --release --features sftp-ota
cargo xtask waveshare-esp32-s3-touch-lcd-43 build --release --features can-no-ack
cargo xtask esp32c6-devkitc build --timings
cargo xtask esp32c6-devkitc tree -i esp-hal
```

You do not need to remember which targets need `+esp` or `-Zbuild-std`:
that lives in [`xtask/src/board.rs`](../xtask/src/board.rs), and
`cargo xtask list` prints the known boards.

## Flashing

`run` builds, flashes and opens the serial monitor (via the `runner`
configured per target triple in `.cargo/config.toml`):

```
cargo xtask esp32c6-devkitc run --release
```

## The BL616 port

`sipeed-m0s-dock` is the odd one out. Its radio comes from
[bl616-radio-reveng](https://github.com/brainstorm/bl616-radio-reveng), which
links Bouffalo's closed 802.11 blobs, so building it needs a BouffaloSDK
checkout and the vendor's RISC-V GCC:

```
BL_SDK_BASE=/path/to/bouffalo_sdk cargo xtask sipeed-m0s-dock build
```

The `bl616-*` crates come from that repository as a git dependency pinned to a
revision in the root `Cargo.toml`. Bump the revision there to take a newer
one. Nothing in CI builds this board — no runner has the SDK — so the
documentation job excludes `ssh-stamp-bl616` as well.

To work on both repositories at once, override the git source with your local
checkout. Put it in a `.cargo/config.toml` **above** both, or in
`~/.cargo/config.toml`, so it stays out of this repository:

```toml
paths = [
    "/path/to/bl616-radio-reveng/bl616-wifi",
    "/path/to/bl616-radio-reveng/bl616-wifi-sys",
    "/path/to/bl616-radio-reveng/bl616-crypto",
    "/path/to/bl616-radio-reveng/bl616-dhcp",
    "/path/to/bl616-radio-reveng/bl616-link",
]
```

## Everything CI checks

```
cargo xtask esp32c6-devkitc build --release    # one CI job per board and chip
cargo xtask esp32c6-devkitc clippy --release -- -D warnings
cargo clippy -p xtask --all-targets -- -D warnings
cargo fmt --all -- --check
cargo xtask esp32c6-devkitc doc --no-deps --lib --workspace --exclude xtask --exclude ssh-stamp-bl616
cargo test              # host-side crates, scoped by workspace default-members
```

Docs build against a board so the build script generates a real pin layout
for the crate front pages.

## Adding a board

1. Drop a `boards/<name>.toml` into the relevant BSP crate, with the pin map
   and a `[build]` section naming its chip:
   ```toml
   [pins]
   uart_rx = 10
   uart_tx = 11
   # can_tx / can_rx and i2c_sda / i2c_scl for the buses the board breaks
   # out; leave out the ones it does not, they show up as `x` in the catalog

   [build]
   chip = "esp32c6"
   # features = ["can"]   # optional: features this board always needs
   ```
2. Add the matching `board-<name>` feature in that platform's `Cargo.toml`.
3. Register the board in `BOARDS` in
   [`xtask/src/board.rs`](../xtask/src/board.rs) using the name, feature, target
   triple, toolchain and RAM windows. This is what `cargo xtask list` will use.  
   Add it to the matrix in `.github/workflows/build.yml` if it should get its own CI job.

The `build.rs` validates the feature against the TOML files, and the
pin layout on the crate's documentation front page is regenerated from
them on the next doc build.

## Adding a chip

Add a `Chip` entry to `CHIPS` in [`xtask/src/board.rs`](../xtask/src/board.rs) with its target triple and
toolchain.
