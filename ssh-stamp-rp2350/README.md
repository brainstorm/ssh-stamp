<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# ssh-stamp-rp2350

RP2350 / Pico 2 port of `ssh-stamp` for the [W6300-EVB-Pico2]: SSH in,
UART out, over **wired Ethernet** via the onboard WIZnet W6300.

[W6300-EVB-Pico2]: https://docs.wiznet.io/Product/Chip/Ethernet/W6300/w6300-evb-pico2

> **Status: builds and links; not yet verified on hardware.** The image
> layout is checked (see [Verified](#what-is-actually-verified)), but no
> byte has moved over a real SPI bus. Treat the SPI clock and the W6300
> bring-up as unproven until someone reports back.

## Prerequisites

```
rustup target add thumbv8m.main-none-eabihf
cargo install probe-rs-tools --locked   # SWD flashing + logs
# or, for drag-and-drop flashing without a probe:
cargo install elf2uf2-rs --locked
```

## Build and flash

Builds go through the workspace's `xtask` runner, same as every other
target; `w6300-evb-pico2` is the board name it knows this PCB by (`cargo
xtask list` shows the rest).

```
cargo xtask build w6300-evb-pico2       # release build
cargo xtask run w6300-evb-pico2         # flash + attach via probe-rs (SWD)
```

Logs come out of the Pico 2's **USB port as a CDC serial device**, so no
debug probe is required just to watch it boot:

```
picocom /dev/ttyACM0        # any baud rate
```

There is a deliberate 2-second pause after boot so the host has time to
enumerate the port before the first log line.

Without a probe, flash over BOOTSEL: hold BOOTSEL while plugging USB, then
convert and copy. Note `elf2uf2-rs` may tag blocks with the RP2040 family
ID, which the RP2350 bootloader rejects — `picotool load` avoids that
entirely and is the safer path:

```
picotool load -x target/thumbv8m.main-none-eabihf/release/ssh-stamp-rp2350
```

## Pinout

Pins live in the [`ssh-stamp-rp2350-boards`](../ssh-stamp-rp2350-boards)
BSP crate — same arrangement as `ssh-stamp-esp32-boards`. Each PCB is one
`boards/*.toml`, and `build.rs` generates the `take_uart_pins!`,
`take_ethernet_pins!` and `select_board!` macros the binary uses; nothing
here hard-codes a GPIO number. `cargo xtask doc` renders the generated
board catalog.

| Signal         | GPIO | How it is driven                              |
|----------------|------|-----------------------------------------------|
| UART0 TX / RX  | 0/1  | hardware UART — this is the bridged serial line |
| W6300 INTn     | 15   | GPIO input, pull-up; driver waits on the edge |
| W6300 CSn      | 16   | GPIO output, toggled by `ExclusiveDevice`     |
| W6300 SCLK     | 17   | PIO side-set                                  |
| W6300 IO0/MOSI | 18   | PIO `OUT`                                     |
| W6300 IO1/MISO | 19   | PIO `IN`                                      |
| W6300 IO2/IO3  | 20/21| **unused** — single-SPI only, see below       |
| W6300 RSTn     | 22   | GPIO output, pulsed by the driver at init     |

Adding another RP-based board means dropping in a TOML plus a
`board-<name>` feature — no Rust changes. One difference from the ESP32
BSP, documented in that crate: its macros hand back embassy-rp's concrete
`PIN_n` singletons instead of erasing to `AnyPin`, because embassy-rp
constrains UART pins with per-instance `TxPin`/`RxPin` traits that
`AnyPin` does not implement.

### Why PIO instead of hardware SPI

The board's `(CS, SCK, IO0, IO1) = (16, 17, 18, 19)` does not match the
RP2350's SPI0 function map (`16=RX, 17=CSn, 18=SCK, 19=TX`). PIO can put
SPI on arbitrary pins; the hardware block cannot, short of rewiring.

### Why single SPI, not QSPI

The board's headline feature is quad SPI (>80 Mbps). `embassy-net-wiznet`
0.3 implements the W6300 in **single-SPI mode only** — quad support is
[embassy-rs/embassy#4662] (PR #5809), still open. So IO2/IO3 stay
unconfigured and the bus runs single-bit at 8 MHz. That is a throughput
ceiling, not a correctness problem: an SSH terminal bridge needs kilobits.
Raising `W6300_SPI_CLK_HZ` in `net.rs` is the first easy win once a link
is confirmed.

[embassy-rs/embassy#4662]: https://github.com/embassy-rs/embassy/issues/4662

## Flash map

```
0x10000000  .vector_table
0x10000114  .start_block      <- RP2350 IMAGE_DEF, must be in first 4 KiB
0x10000128  .text / .rodata   (~465 KiB today)
   ...
0x103FF000  ssh-stamp config  <- 4 KiB, one erase sector
0x10400000  end of declared 4 MiB
```

`ssh_stamp::store` addresses the config at a fixed `CONFIG_OFFSET`
(`0x9000`, an ESP partition convention) which on this chip would land in
our own program text. `flash.rs` therefore exposes a *translating view*
that remaps that window onto the top sector and rejects every access
outside it, so a store bug cannot scribble on the firmware.

## First boot

No WiFi AP fallback exists here — DHCP is the only way in.

1. `cargo xtask run w6300-evb-pico2`, watch USB CDC for `W6300: IPv4 <addr>`.
2. `ssh -o SendEnv=SSH_STAMP_PUBKEY root@<addr>` with `SSH_STAMP_PUBKEY`
   set to your public key. The host key fingerprint is printed at boot;
   compare it on first connect.

The MAC is minted from the TRNG on first boot and persisted with the rest
of the config, so it is stable across reboots.

## What is actually verified

- Compiles and links clean for `thumbv8m.main-none-eabihf`; clippy clean
  at `-D warnings`.
- `.start_block` lands at `0x10000114`, inside the first 4 KiB where the
  boot ROM looks for it (`readelf -S`).
- Image ends around `0x10071000`, far below the config sector.
- Entropy comes from the RP2350 TRNG (`CryptoRng`), not the ROSC counter —
  it generates the SSH host key.

## What is not

- **Nothing has run on hardware.** No SPI transaction, no link, no DHCP.
- The 8 MHz PIO SPI clock is a guess. If the driver's `VERSIONR` check
  fails at init you will see `W6300 init failed`: drop the clock, confirm
  wiring, then climb back up.
- The W6300 driver disables the MAC address filter (upstream found DHCP
  fails with it on), so the MCU sees all bus traffic and filters in
  software. Expect more interrupt load on a busy LAN than a W5500 would.
- **OTA is not implemented.** Staging would be easy; *activating* is not,
  since the boot ROM selects images from a partition table this port does
  not define. Every `OtaActions` method refuses rather than accepting an
  upload that could never boot. Reflash over USB or SWD.
