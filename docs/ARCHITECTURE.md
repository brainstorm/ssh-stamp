# ARCHITECTURE.md

## Bird's Eye View

SSH-Stamp is firmware for turning microcontrollers into SSH-accessible serial bridges. You connect via SSH to the device, and your terminal session is bridged directly to the device's UART—perfect for debugging embedded systems remotely.

The firmware runs on ESP32 family microcontrollers (ESP32, ESP32-C3, ESP32-C6, ESP32-S2, ESP32-S3) and creates a WiFi access point. When you SSH into the device, you get a shell that's directly connected to the target's serial port.

## Architecture Philosophy

**Platform-agnostic core, platform-specific HAL.** The main firmware (`src/`) knows nothing about ESP32 hardware. All hardware concerns flow through the Hardware Abstraction Layer (`hal/`). This separation means porting to other microcontrollers (i.e: Nordic nRF, STM32, RP2040) requires only implementing the HAL traits in a new `hal-*` crate.

**Traits define contracts.** Each peripheral category (UART, WiFi, Flash, etc.) has a trait in `hal/src/traits/`. Implementations live in platform-specific crates like `hal-espressif/`.

**State machine drives the application.** The firmware uses a Hierarchical State Machine (HSM). States like "WaitingForWiFi", "SSHConnected", and "Bridging" encode what's configured and what's running at any moment.

## Crate Structure

```
ssh-stamp/
├── hal/                    # Platform-agnostic trait definitions
├── hal-espressif/          # ESP32 implementation of hal traits
├── ota/                    # SFTP-based OTA update server
└── src/                    # Main firmware (platform-agnostic)
```

### `hal/` — Trait Definitions

The `hal` crate defines *what* a hardware platform must provide, not *how*. It contains traits and configuration structs, no implementations.

Key entities:
- `HalPlatform` — Bundles all peripherals together. Entry point for hardware initialization.
- `UartHal` — Async read/write for serial communication.
- `WifiHal` — WiFi access point creation and TCP socket management.
- `FlashHal` — Raw flash read/write/erase operations.
- `OtaActions` — OTA partition management (validate, write, finalize, reset).
- `HashHal` — HMAC-SHA256 for secure operations.
- `RngHal` — Hardware random number generation.
- `ExecutorHal` — Async runtime with interrupt priority management.

Files to read first:
- `hal/src/lib.rs` — The `HalPlatform` trait that ties everything together.
- `hal/src/traits/uart.rs` — Example of a simple trait.
- `hal/src/traits/network/wifi.rs` — Example of a more complex trait with associated types.

### `hal-espressif/` — ESP32 Implementation

Implements all `hal` traits for ESP32 family chips using `esp-hal`, `esp-radio`, and related crates.

Key entities:
- `EspHalPlatform` — Implements `HalPlatform`. Contains the full hardware bundle.
- `EspUart` — Uses `esp-hal::uart` with DMA buffering.
- `EspWifi` — Uses `esp-radio::wifi` for AP mode.
- `EspFlash` — Uses `esp-storage` for flash access.

Files to read first:
- `hal-espressif/src/lib.rs` — See how peripherals are bundled.
- `hal-espressif/src/config.rs` — Per-target default UART/WiFi pin configurations.

### `ota/` — OTA Update Server

Platform-agnostic SFTP server implementation that receives firmware updates. Uses `OtaActions` trait from `hal` to write to flash and reboot.

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

**No direct hardware dependencies in `src/`.** The main firmware crate never imports `esp-hal`, `esp-radio`, or similar. All hardware access goes through `hal::HalPlatform` or its trait methods.

**State machine owns all peripherals.** Peripherals are passed into the HSM at initialization and flow through states. No global mutable state for hardware resources.

**UART is exclusive resource.** Only one `UartHal` instance exists. The serial bridge takes ownership when SSH session starts.

**Flash operations are async.** All flash read/write/erase operations return futures. The `FlashHal` trait abstracts platform-specific blocking or non-blocking implementations.

**OTA is optional.** The `sftp-ota` feature flag enables SFTP-based firmware updates. Without it, the firmware is smaller and simpler.

## Boundaries Between Layers

### `hal/` ↔ `hal-espressif/`

The boundary is defined by traits. `hal/` contains trait definitions and configuration structs. `hal-espressif/` contains implementations. Adding a new platform (e.g., `hal-nordic/`) requires implementing all traits but no changes to `hal/`.

### `hal` ↔ `ota`

The `ota` crate depends on `hal::OtaActions` trait. It knows nothing about flash partitions, ESP-IDF, or ESP-specific OTA. The platform implementation (`hal-espressif/src/flash.rs`) handles partition management.

### `src/handle.rs` ↔ `sunset`

`handle.rs` contains handlers for `ServEvent` enums from the `sunset` SSH library. The boundary is defined by the `ServEvent` type. Handlers extract SSH payloads and route to appropriate subsystems (config update, UART bridge, etc.).

## Cross-Cutting Concerns

### Error Handling

All HAL operations return `Result<T, HalError>`. The `HalError` enum in `hal/src/error.rs` aggregates all platform error types. Platform-specific error details are converted to common variants (e.g., `UartError::Read` → `HalError::Uart(UartError::Read)`).

### Async Runtime

Uses `embassy-executor` for async task scheduling. The `ExecutorHal` trait provides access to the spawner and handles interrupt priority configuration. ESP32 uses `esp-rtos` as the Embassy runtime backend.

### Logging

Uses `log` crate facade. Platform implementations wire up appropriate backends (`esp-println` for ESP32). Log levels are configurable at compile time.

### Feature Flags

Root `Cargo.toml` defines unified feature flags:
- `target-esp32`, `target-esp32c6`, etc. — Select target MCU
- `sftp-ota` — Enable SFTP-based OTA updates

Target selection propagates to `hal-espressif` which enables corresponding `esp-hal` features.

## Adding New Hardware Support

To port to a new microcontroller family:

1. Create `hal-yourplatform/` crate
2. Implement all traits from `hal/src/traits/`
3. Create `YourPlatformHalPlatform` struct implementing `HalPlatform`
4. Add feature flag in root `Cargo.toml`
5. Add per-target default configs in `hal-yourplatform/src/config.rs`

No changes needed in `src/` or `hal/`.

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

1. Add field to `UartConfig` in `hal/src/config.rs`
2. Update `UartHal` trait if needed (`hal/src/traits/uart.rs`)
3. Implement in `hal-espressif/src/uart.rs`
4. Update per-target defaults in `hal-espressif/src/config.rs`

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
| Hardware traits | `hal/src/traits/*.rs` |
| ESP32 implementations | `hal-espressif/src/*.rs` |
| Default pin configs | `hal-espressif/src/config.rs` |
| Configuration struct | `src/config.rs` |
| Serial bridge logic | `src/serial.rs` |
| OTA server | `ota/src/lib.rs` |

## Dependencies

Key external crates:
- `sunset` / `sunset-async` — SSH protocol implementation
- `embassy-sync` / `embassy-executor` — Async primitives and runtime
- `esp-hal` — ESP32 peripheral access (only in `hal-espressif/`)
- `esp-radio` — ESP32 WiFi (only in `hal-espressif/`)
- `heapless` — No-std collections (String, Vec)
- `heapless` — Stack-allocated collections

## Testing

Currently no automated tests. Manual testing requires:
1. Hardware target (ESP32 dev board)
2. WiFi client to connect to AP
3. SSH client to test connection
4. Serial device connected to UART pins for bridge testing

Focus on architecture correctness over test coverage for now.