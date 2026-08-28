#!/usr/bin/env bash
set -eo pipefail

cd "$(dirname "$0")/.."

cargo expand --release --target riscv32imac-unknown-none-elf --no-default-features --features board-esp32c6-devkitc -p ssh-stamp-esp32 --bin ssh-stamp-esp32 --color never > expanded/bin_esp32c6.rs

cargo expand --release --target riscv32imac-unknown-none-elf --no-default-features --features board-esp32c6-devkitc -p ssh-stamp-esp32 --lib --color never > expanded/lib_esp32c6.rs

cargo expand --release --target riscv32imac-unknown-none-elf --no-default-features --features board-esp32c5-devkitc -p ssh-stamp-esp32 --bin ssh-stamp-esp32 --color never > expanded/bin_esp32c5.rs

cargo expand --release --target riscv32imac-unknown-none-elf --no-default-features --features board-esp32c5-devkitc -p ssh-stamp-esp32 --lib --color never > expanded/lib_esp32c5.rs
