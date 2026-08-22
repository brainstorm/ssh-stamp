<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

# Running ssh-stamp without hardware

Espressif's own [esp-emulator] runs ssh-stamp real firmware,
associates over emulated WiFi, and serves a complete SSH handshake to the
host — no board attached.

Unfortunately this approach is Espressif-specific, we'll have to decide
what approach to follow in the future w.r.t other targets.

## ESP32C6 example 

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

That is the whole boot path for integration testing purposes.

## Emulation issues

This emulation does not replace HIL and there's a couple of gotchas.

### MLKEM cannot be emulated efficiently

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

### Emulator as AP

The emulators WiFi model expects firmware to behave as a *station* joining `--wifi-ssid`.

A factory ssh-stamp boots as an AP instead, and nothing on the host can associate with it, so
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

## Using the esp-emulator with SSH Stamp

```sh
curl -fsSL https://raw.githubusercontent.com/espressif/esp-emulator/main/install.sh | sh
./test/emulator/run-esp32c6.sh --ssh-port 2222
```

The script builds the firmware, merges bootloader + partition table + app into
a single flash image with `espflash save-image --merge` (no ESP-IDF needed,
despite the upstream docs describing an `idf.py merge-bin` flow), and boots it.

## Future directions

- Smoke test in CI: boot, assert on `--exit-on "SSH hostkey fingerprint"`,
  fail on timeout. Cheap, and catches boot regressions no unit test would.
- Config-partition fixture (above), unlocking station mode and therefore full
  SSH integration tests against a real client.
- The `roundtrip_config` `sshwire` test from the issue thread is a plain host
  unit test and needs none of this — it belongs in `src/config.rs`.
- Untried: `--inject`/`--inject-on` drive UART RX, which is the other half of
  the bridge and looks like the natural way to test the UART side.

[esp-emulator]: https://github.com/espressif/esp-emulator
