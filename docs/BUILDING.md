# Building

Tooling is controlled by `rust-toolchain.toml`. On a fresh host you'll typically need the Rust source component and a flasher (we use `espflash` below as an example):

```
rustup toolchain install stable --component rust-src
cargo install espflash --locked
```

Build/flash for your board using the short command pattern (replace `<target>` with the concrete chip you have):

| Machine target | Rust toolchain target |
| --- | --- |
| `esp32` | `xtensa-esp32-none-elf` |
| `esp32c2` | `riscv32imc-unknown-none-elf` |
| `esp32c3` | `riscv32imc-unknown-none-elf` |
| `esp32c5` | `riscv32imac-unknown-none-elf` |
| `esp32c6` | `riscv32imac-unknown-none-elf` |
| `esp32c61` | `riscv32imac-unknown-none-elf` |
| `esp32s2` | `xtensa-esp32s2-none-elf` |
| `esp32s3` | `xtensa-esp32s3-none-elf` |

```
rustup target add <rust-toolchain-target>
cargo build-<machine-target>     # e.g. cargo build-esp32c6, cargo build-esp32c3, cargo build-esp32
cargo run-<machine-target>       # convenience helper (if supported) that builds + flashes
```

Xtensa targets (ESP32/ESP32-S2/S3) do require `espup` in addition to the `rustup` command above:

```
cargo install espup
espup install
source $HOME/export-esp.sh
```

# Flashing

Flash the firmware and open the serial console (example):

```
# build & flash (example for esp32c6)
cargo build-esp32c6 --release
cargo run-esp32c6
```