<!--
SPDX-FileCopyrightText: 2026 Marko Malenic <mmalenic1@gmail.com>

SPDX-License-Identifier: GPL-3.0-or-later
-->

Testing for ssh-stamp comes in two forms, unit tests and HIL tests. 

# Unit tests

The [`ota`](../ota), [`xtask`](../xtask) and parts of the [`ssh-stamp`](../src) core
library have unit tests. These can be run directly on the host:

```sh
cargo test
```

# HIL tests

There is also a set of HIL tests that require an ESP32 board to run. These
are built on the [embedded-test] framework as part of the
[ssh-stamp-esp32-hil](../ssh-stamp-esp32-hil) crate. Each file under the 
`tests/`directory there compiles it's own test binary, which is flashed by
probe-rs and run.

[embedded-test]: https://crates.io/crates/embedded-test

## Requirements

- The probe-rs CLI: `cargo install probe-rs-tools`.
- A debug connection to the chip.

## Running

```sh
cargo xtask esp32c6-devkitc test
```

Any board or chip target that `cargo xtask` accepts works in place of
`esp32c6-devkitc`. The tests always build with the release profile.

## Effects on the board

- Running the tests replaces the application in flash, reflash the
  firmware afterwards: `cargo xtask <board> run`.
- `tests/flash.rs` overwrites the stored config sector. This test accepts
  chip targets as well as board targets.
- `tests/uart.rs` uses the board's UART TX pin, looped back into the RX
  internally. This test only accepts board targets.

## Adding tests

Each `tests/*.rs` file is a separate flash and run. A new file needs a
`[[test]]` entry with `harness = false` in `Cargo.toml`.
