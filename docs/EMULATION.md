<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# Running ssh-stamp without hardware

Issue #37 wants automated testing, and issue #37's comment thread asks whether
anything can stand in for [Wokwi] — whose CI use would need an MoU
re-negotiation. This is the result of evaluating the candidates.

**Summary: neither suggested substitute fits, but Espressif's own
[esp-emulator] does, and it runs ssh-stamp today.** Real firmware boots,
associates over emulated WiFi, and serves a complete SSH handshake to the
host — no board attached.

[Wokwi]: https://wokwi.com/
[esp-emulator]: https://github.com/espressif/esp-emulator

## What was evaluated

| | licence | chips | firmware it accepts | headless / CI |
|---|---|---|---|---|
| [velxio] | AGPL-3.0 + commercial | AVR, RP2040, ESP32-C3, ESP32/S3, Pi 3 | compiles Arduino sketches; **does not run precompiled ELF** | browser-first, self-host via Docker |
| [Cirkit ESP32-S3] | not stated / proprietary | **ESP32-S3 only** | Arduino sketches compiled **on their server** | browser only, no CLI or API |
| [esp-emulator] | **Apache-2.0** | C3, **C6**, H2, P4 (S3 early) | any merged flash image | **yes** — `--timeout`, `--exit-on`, `--inject` |

[velxio]: https://github.com/davidmonterocrespo24/velxio
[Cirkit ESP32-S3]: https://www.cirkitdesigner.com/blog/2026-05-05-esp32-s3-simulator

Both suggested substitutes are ruled out for the same underlying reason: they
are Arduino-sketch playgrounds, not firmware emulators. ssh-stamp is a Rust
`no_std` ELF, so "paste a sketch and we compile it" cannot load it at all.
Neither covers the ESP32-C6, which is the default board. Cirkit additionally
compiles on its own servers and offers no CLI, so it is both unusable in CI and
unattractive for a security device — it would mean uploading firmware sources
to a third party, which is a worse position than the Wokwi MoU question that
prompted the search.

`esp-emulator` (binary `esp-emu`) is a first-party Espressif RISC-V emulator
written in Rust. It is instruction-accurate, Apache-2.0, headless, and covers
the ESP32-C6 — including WiFi soft-AP with WPA2, a DHCP server, and QEMU-style
user-mode networking with port forwarding.

## What actually works

Verified on `esp-emu` 0.38.0, ESP32-C6, ssh-stamp at sunset 0.6:

- **Boot.** The ESP-IDF second-stage bootloader runs, our `partitions.csv` is
  parsed, and the app is loaded from `ota_0`.
- **Entropy and key generation.** First boot mints a WiFi PSK and an ed25519
  host key, so the TRNG / `getrandom` custom backend works under emulation.
- **Flash config.** The generated config persists to the config partition.
- **WiFi.** In station mode the firmware associates with the emulator's soft AP
  (`authmode: Wpa2Personal`) and takes a DHCP lease on `192.168.4.2`.
- **SSH.** With `hostfwd`, an `ssh` client on the host completes a full
  handshake, and the server host key it presents matches the fingerprint the
  firmware logged at boot.

```
INFO - SSH server ident: SSH-2.0-Sunset-0.6.0-ssh-stamp-0.3.0
INFO - SSH hostkey fingerprint: SHA256:VO+Yvf+tm7o39TlcybTUPNv5NAxc/iQFlt9fjrekW6g
INFO - Wifi connected to ConnectedInfo { ssid: "myssid", ..., authmode: Wpa2Personal }
INFO - Connect to the AP `myssid` with IP 192.168.4.2/24
```

```
debug1: kex: algorithm: curve25519-sha256
debug1: Server host key: ssh-ed25519 SHA256:VO+Yvf+tm7o39TlcybTUPNv5NAxc/iQFlt9fjrekW6g
Authenticated to 127.0.0.1 ([127.0.0.1]:2223)
```

That is the whole boot path an integration test cares about, reachable from a
CI runner with no board attached.

## Two things that will bite

**The default key exchange is too slow to emulate.** sunset offers
`mlkem768x25519-sha256` first, and a client that picks it never gets past the
banner — over seven minutes with no progress. Forcing the classical exchange
completes promptly:

```
ssh -o KexAlgorithms=curve25519-sha256 -p 2222 root@127.0.0.1
```

This is emulation speed, not a firmware bug; post-quantum KEX is simply
expensive when every instruction is interpreted. Any emulator-based test must
pin the KEX. It also means the post-quantum path still needs real hardware to
exercise, so this does not replace HIL testing.

**The emulator is the access point, not the device.** Its WiFi model expects
firmware to behave as a *station* joining `--wifi-ssid`. A factory ssh-stamp
boots as an AP instead, and nothing on the host can associate with it, so
`hostfwd` has nothing to forward to — the port simply refuses. The addresses
make this easy to misdiagnose: the emulator's gateway is `192.168.4.1`, which
is also the address ssh-stamp uses for its own AP.

Reaching SSH therefore needs the config to carry station credentials before
boot. The experiment above did that by temporarily defaulting
`wifi_sta_ssid`/`wifi_sta_pw` in `SSHStampConfig::new`, which is fine for a
one-off but not for CI. The clean fix is a host-side fixture that writes an
`sshwire`-encoded config into the image's config partition at `CONFIG_OFFSET`
before boot — worth building if this becomes a CI job, and useful beyond
emulation.

## Reproducing

```sh
curl -fsSL https://raw.githubusercontent.com/espressif/esp-emulator/main/install.sh | sh
./hil/emulator/run-esp32c6.sh --ssh-port 2222
```

The script builds the firmware, merges bootloader + partition table + app into
a single flash image with `espflash save-image --merge` (no ESP-IDF needed,
despite the upstream docs describing an `idf.py merge-bin` flow), and boots it.

## Where this could go

- Smoke test in CI: boot, assert on `--exit-on "SSH hostkey fingerprint"`,
  fail on timeout. Cheap, and catches boot regressions no unit test would.
- Config-partition fixture (above), unlocking station mode and therefore full
  SSH integration tests against a real client.
- The `roundtrip_config` `sshwire` test from the issue thread is a plain host
  unit test and needs none of this — it belongs in `src/config.rs`.
- Untried: `--inject`/`--inject-on` drive UART RX, which is the other half of
  the bridge and looks like the natural way to test the UART side.
