<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# ARCHITECTURE.md

## Bird's Eye View

SSH-Stamp is firmware for turning microcontrollers into SSH-accessible serial bridges. You connect via SSH to the device, and your terminal session is bridged directly to the device's UART—perfect for debugging embedded systems remotely.

The firmware runs on ESP32 family microcontrollers (ESP32, ESP32-C3, ESP32-C6, ESP32-S2, ESP32-S3) and creates a WiFi access point. When you SSH into the device, you get a shell that's directly connected to the target's serial port.

## Architecture Philosophy

**Platform-agnostic core, platform-specific HAL.** The main firmware (`src/`) knows nothing about ESP32 hardware. All hardware concerns flow through the Hardware Abstraction Layer (`ssh-stamp-hal/`). This separation means porting to other microcontrollers (i.e: Nordic nRF, STM32, RP2040) requires only implementing the HAL traits in a new `ssh-stamp-*` crate.

**Fine-grained, composable traits.** Following the [embedded-hal](https://github.com/rust-embedded/embedded-hal) pattern, each peripheral has its own trait. There is no monolithic `HalPlatform` bundle — new MCU ports implement only the traits they support. Standard peripheral operations defer to ecosystem crates (`embedded-storage-async`, `embedded-io-async`) where possible.

**State machine drives the application.** The firmware uses a Hierarchical State Machine (HSM). States like "WaitingForWiFi", "SSHConnected", and "Bridging" encode what's configured and what's running at any moment.

## Crate Structure

```
ssh-stamp/
├── ssh-stamp-hal/           # Platform-agnostic trait definitions
├── ssh-stamp-esp32/         # ESP32 implementation of ssh-stamp-hal traits
├── ota/                     # SFTP-based OTA update server
└── src/                     # Main firmware (platform-agnostic)
```

### `ssh-stamp-hal/` — Trait Definitions

The `ssh-stamp-hal` crate defines *what* a hardware platform must provide, not *how*. It contains traits and configuration structs, no implementations.

Key entities:
- `UartHal` — Async read/write for serial communication.
- `WifiHal` — WiFi access point creation and TCP socket management.
- `OtaActions` — OTA partition management (validate, write, finalize, reset).
- `HashHal` — HMAC-SHA256 for secure operations.
- `RngHal` — Hardware random number generation.
- `TimerHal` — Async timer/delay operations.

Flash storage operations use `embedded-storage-async::NorFlash` from the embedded-hal ecosystem directly, rather than a custom trait.

Files to read first:
- `ssh-stamp-hal/src/lib.rs` — Re-exports and crate overview.
- `ssh-stamp-hal/src/traits/uart.rs` — Example of a simple trait.
- `ssh-stamp-hal/src/traits/flash.rs` — Example of an application-specific trait (`OtaActions`).

### `ssh-stamp-esp32/` — ESP32 Implementation

Implements `ssh-stamp-hal` traits for ESP32 family chips using `esp-hal`, `esp-radio`, and related crates.

Key entities:
- `EspUart` — Uses `esp-hal::uart` with DMA buffering.
- `EspWifi` — Uses `esp-radio::wifi` for AP mode.
- `EspOtaWriter` — Uses `esp-storage` and `esp-bootloader-esp-idf` for OTA partition management.
- `reset()` / `mac_address()` — Direct functions for hardware operations.

Files to read first:
- `ssh-stamp-esp32/src/lib.rs` — Re-exports, `reset()`, `mac_address()`.
- `ssh-stamp-esp32/src/config.rs` — Per-target default UART/WiFi pin configurations.

### `ota/` — OTA Update Server

Platform-agnostic SFTP server implementation that receives firmware updates. Uses `OtaActions` trait from `ssh-stamp-hal` to write to flash and reboot.

Key entities:
- `OtaWriter` — Implements `OtaActions` for the platform.
- `run_ota_server()` — Main OTA server loop.

Files to read first:
- `ota/src/lib.rs` — Public API.

### `src/` — Main Firmware

Platform-agnostic application code. Contains the HSM state machine, SSH handling, and configuration management.

Key entities:
- `main.rs` — HSM state definitions and transitions. Start here.
- `handle.rs` — SSH event handlers (authentication, channels, environment variables).
- `serve.rs` — SSH connection loop orchestration.
- `config.rs` — Configuration struct stored in flash.
- `serial.rs` — Serial bridge logic (SSH ↔ UART).

Files to read first:
- `src/main.rs` — See the HSM states and how they transition.
- `src/handle.rs` — Understand what happens when you send SSH commands.

## Configuration System

Configuration is stored in flash and loaded at boot:

```
SSHStampConfig {
    hostkey: Ed25519PrivateKey,    // Generated on first boot
    pubkeys: [Option<Ed25519PublicKey>; N],  // Allowed SSH public keys
    wifi_ssid: String,             // AP name
    wifi_pw: Option<String>,        // AP password (None = open)
    mac: [u8; 6],                   // MAC address (or 0xFF for random)
    first_login: bool,              // First boot? Enables key provisioning
}
```

On first boot (`first_login = true`), the device accepts any SSH connection with an empty password. The client sends their public key via SSH environment variables, which gets stored. Subsequent connections require that key for authentication.

## Architectural Invariants

**No direct hardware dependencies in `src/`.** The main firmware crate never imports `esp-hal`, `esp-radio`, or similar. All hardware access goes through `ssh-stamp-hal` traits or `ssh-stamp-esp32` functions.

**State machine owns all peripherals.** Peripherals are passed into the HSM at initialization and flow through states. No global mutable state for hardware resources.

**UART is exclusive resource.** Only one `UartHal` instance exists. The serial bridge takes ownership when SSH session starts.

**Flash operations are async.** All flash read/write/erase operations return futures. Flash uses `embedded-storage-async::NorFlash` from the embedded-hal ecosystem.

**OTA is optional.** The `sftp-ota` feature flag enables SFTP-based firmware updates. Without it, the firmware is smaller and simpler.

## Boundaries Between Layers

### `ssh-stamp-hal/` ↔ `ssh-stamp-esp32/`

The boundary is defined by traits. `ssh-stamp-hal/` contains trait definitions and configuration structs. `ssh-stamp-esp32/` contains implementations. Adding a new platform (e.g., `ssh-stamp-nrf/`) requires implementing the needed traits but no changes to `ssh-stamp-hal/`.

### `ssh-stamp-hal` ↔ `ota`

The `ota` crate depends on `ssh_stamp_hal::OtaActions` trait. It knows nothing about flash partitions, ESP-IDF, or ESP-specific OTA. The platform implementation (`ssh-stamp-esp32/src/flash.rs`) handles partition management.

### `src/handle.rs` ↔ `sunset`

`handle.rs` contains handlers for `ServEvent` enums from the `sunset` SSH library. The boundary is defined by the `ServEvent` type. Handlers extract SSH payloads and route to appropriate subsystems (config update, UART bridge, etc.).

## Cross-Cutting Concerns

### Error Handling

All HAL operations return `Result<T, HalError>`. The `HalError` enum in `ssh-stamp-hal/src/error.rs` aggregates all platform error types. Platform-specific error details are converted to common variants (e.g., `UartError::Read` → `HalError::Uart(UartError::Read)`).

### Async Runtime

Uses `embassy-executor` for async task scheduling. Embassy is used across all MCU targets, so no HAL trait is needed for it. ESP32 uses `esp-rtos` as the Embassy runtime backend.

### Logging

Uses `log` crate facade. Platform implementations wire up appropriate backends (`esp-println` for ESP32). Log levels are configurable at compile time.

### Feature Flags

Root `Cargo.toml` defines unified feature flags:
- `esp32`, `esp32c6`, etc. — Select target MCU
- `sftp-ota` — Enable SFTP-based OTA updates

Target selection propagates to `ssh-stamp-esp32` which enables corresponding `esp-hal` features.

## Adding New Hardware Support

To port to a new microcontroller family:

1. Create `ssh-stamp-yourplatform/` crate
2. Implement the needed traits from `ssh-stamp-hal/src/traits/`
3. Provide `reset()` and `mac_address()` functions
4. Add feature flag in root `Cargo.toml`
5. Add per-target default configs in `ssh-stamp-yourplatform/src/config.rs`

No changes needed in `src/` or `ssh-stamp-hal/`. You only implement the traits your platform supports — missing peripherals are gated by feature flags in the app.

## Common Tasks

### Adding a new SSH environment variable handler

Edit `src/handle.rs`. Find `session_env()` function. Add a new match arm for your variable name:

```rust
"SSH_STAMP_NEW_VAR" => {
    let mut config_guard = config.lock().await;
    // Handle the new variable
    a.succeed()?;
    *ctx.config_changed = true;
}
```

### Adding a new UART configuration option

1. Add field to `UartConfig` in `ssh-stamp-hal/src/config.rs`
2. Update `UartHal` trait if needed (`ssh-stamp-hal/src/traits/uart.rs`)
3. Implement in `ssh-stamp-esp32/src/uart.rs`
4. Update per-target defaults in `ssh-stamp-esp32/src/config.rs`

### Adding a new HSM state

1. Define state struct in `src/main.rs` implementing a state trait
2. Add transition logic to parent state
3. Handle entry/exit conditions
4. Wire up any event handlers needed

## Key Files Quick Reference

| What | Where |
|------|-------|
| SSH event handling | `src/handle.rs` |
| HSM states | `src/main.rs` |
| Hardware traits | `ssh-stamp-hal/src/traits/*.rs` |
| ESP32 implementations | `ssh-stamp-esp32/src/*.rs` |
| Default pin configs | `ssh-stamp-esp32/src/config.rs` |
| Configuration struct | `src/config.rs` |
| Serial bridge logic | `src/serial.rs` |
| OTA server | `ota/src/lib.rs` |

## Dependencies

Key external crates:
- `sunset` / `sunset-async` — SSH protocol implementation
- `embassy-sync` / `embassy-executor` — Async primitives and runtime
- `embedded-storage-async` — Flash storage traits (embedded-hal ecosystem)
- `esp-hal` — ESP32 peripheral access (only in `ssh-stamp-esp32/`)
- `esp-radio` — ESP32 WiFi (only in `ssh-stamp-esp32/`)
- `heapless` — Stack-allocated collections

## Testing

OTA TLV serialization tests run on host:
```bash
cargo test-ota
```

Manual testing requires:
1. Hardware target (ESP32 dev board)
2. WiFi client to connect to AP
3. SSH client to test connection
4. Serial device connected to UART pins for bridge testing