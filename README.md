# esp-ssh-rs

Your everyday SSH secured serial access

# Building

Rust versions are controlled via `rust-toolchain.toml` and the equivalent defined on the CI workflow.

On a fresh system the following should be enough to build and run on an ESP32-C6 dev board.

```
rustup toolchain install stable --component rust-src
rustup target add riscv32imac-unknown-none-elf
cargo build --release
```

Running on the target:

```
cargo install cargo-espflash
cargo run --release
```
