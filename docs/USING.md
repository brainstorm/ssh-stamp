<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

## First boot & provisioning

Once the flash process finishes successfully, follow the steps below:

1. On first boot the device generates a random WPA2 PSK and prints it to the serial console with the following (or similar) info messages:

```
(...)
INFO - WIFI PSK: <PSK>
INFO - WIFI MAC: <MAC>
INFO - SSH hostkey fingerprint: <FINGERPRINT>
INFO - Connect to the AP `<RANDOM AP NAME>` as a DHCP client with IP: 192.168.4.1
```

2. Connect a laptop/phone to the WiFi AP using the printed SSID and PSK, then SSH into the device at `root@192.168.4.1`.

3. Provisioning via SSH environment variables

You can provision the device by sending these environment variables with your SSH client. Examples below use OpenSSH and `SendEnv` to forward local environment variables to the device.

- Add your public key (first-boot only):

```
export SSH_STAMP_PUBKEY="$(cat ~/.ssh/id_ed25519.pub)"
ssh -o SendEnv=SSH_STAMP_PUBKEY root@192.168.4.1
```

- Set a custom SSID and WPA2 PSK (allowed on first-boot or any authenticated session):

```
export SSH_STAMP_WIFI_AP_SSID="SshStampSSID"
export SSH_STAMP_WIFI_AP_PSK="my-super-secret-psk"
ssh -o SendEnv=SSH_STAMP_WIFI_AP_SSID -o SendEnv=SSH_STAMP_WIFI_AP_PSK root@192.168.4.1
```

- To connect the SSH Stamp to an existing access point with DHCP (Station Mode):
```
export SSH_STAMP_WIFI_STA_SSID="MyHomeSSID"
export SSH_STAMP_WIFI_STA_PSK="my-super-secret-psk"
ssh -o SendEnv=SSH_STAMP_WIFI_STA_SSID -o SendEnv=SSH_STAMP_WIFI_STA_PSK root@192.168.4.1
```

- To return to the default Access Point mode, clear the Station SSID:
```
export SSH_STAMP_WIFI_STA_SSID=""
ssh -o SendEnv=SSH_STAMP_WIFI_STA_SSID root@192.168.4.1
```

- To select the WiFi band (ESP32-C5 only; ignored on other chips):
```
export SSH_STAMP_WIFI_BAND="5g"
ssh -o SendEnv=SSH_STAMP_WIFI_BAND root@192.168.4.1
```
Accepts `2.4g` (default), `5g`, or `auto`. The device resets after applying the change.

Notes:
- `SSH_STAMP_PUBKEY` is accepted on first-boot to add the initial admin key.
- `SSH_STAMP_WIFI_AP_SSID` and `SSH_STAMP_WIFI_AP_PSK` may be applied while authenticated via pubkey (or on first-boot). After a successful change the device persists the settings and performs a software reset so the new WiFi settings take effect.
- `SSH_STAMP_WIFI_BAND` selects the AP radio band. Only the ESP32-C5 supports 5GHz; other chips ignore the setting and stay on 2.4GHz.
- If you prefer a single-step provisioning, export all three env vars locally and forward them with `SendEnv` in the same SSH invocation.

If your SSH client doesn't forward environment variables by default, use the `-o SendEnv=VAR` option as shown above or configure `SendEnv` in your SSH client config.

# Device messages

## Before you log in

The device sends an SSH banner, which your client prints before it tries to
authenticate. On a device that has not been claimed yet:

```
$ ssh root@192.168.4.1
ssh-stamp: first-login provisioning is OPEN - this device accepts any client.
ssh-stamp: claim it by sending SSH_STAMP_PUBKEY.
```

and on one that has been:

```
ssh-stamp: 1 authorised key(s); provisioning is closed.
```

If the stored configuration has provisioning closed but no authorised key —
so nothing could ever log in — the device says so and disconnects with
`SSH_DISCONNECT_NO_MORE_AUTH_METHODS_AVAILABLE`, rather than letting you
hunt for a key problem that does not exist.

Note this needs a sunset with the banner and disconnect send paths; see the
`[patch.crates-io]` section in the workspace `Cargo.toml`.

## Once you are in

A shell session is a transparent pipe to the target UART, so the device
cannot explain itself on stdout without corrupting that stream. It uses SSH
stderr instead, and every line is prefixed `ssh-stamp:`.

```
$ ssh root@192.168.4.1
ssh-stamp: config: wifi ap ssid "ssh-stamp-a1b2" -> "SshStampSSID"
ssh-stamp: config: saved to flash
ssh-stamp: --- configuration ---
ssh-stamp: uart: rx=GPIO17 tx=GPIO16
ssh-stamp: wifi ap: ssid="SshStampSSID" psk=set band=2.4GHz
ssh-stamp: wifi station: not configured
ssh-stamp: mac: 40:4c:ca:12:34:56
ssh-stamp: ipv4: dhcp
ssh-stamp: authorised keys: 1/4
ssh-stamp: ---------------------
ssh-stamp: bridge connected
<target UART output from here on>
```

Because the two streams are separate, redirection picks what you want:

```
ssh root@192.168.4.1 > capture.bin    # UART bytes only, byte-for-byte
ssh root@192.168.4.1 2>/dev/null      # UART bytes only, messages discarded
ssh root@192.168.4.1 2>notes.txt      # both, kept apart
```

If your client merges the streams and you cannot separate them afterwards
(`ssh -t` does this), turn the messages off at the device:

```
export SSH_STAMP_NOTICES=off
ssh -o SendEnv=SSH_STAMP_NOTICES root@192.168.4.1
```

Messages reported this way include: which UART pins are in use, a summary of
the running configuration, every configuration change as `old -> new`, why a
`SSH_STAMP_*` variable was rejected, why an `sftp` or `can` subsystem request
was refused, bridge connect/disconnect, and UART RX overruns — the last of
which tells you a capture has a hole in it.

Two deliberate limits:

- **Secrets are never printed.** The summary reports whether a PSK is set,
  not what it is. An authenticated client could be told, but a PSK echoed
  into a terminal ends up in scroll buffers and pasted bug reports.
- **A change that triggers a reboot cannot be acknowledged.** WiFi changes
  reset the device before the shell channel opens, so the client sees the
  connection drop with no explanation. Reconnect and the summary will show
  the new values.

Messages are queued during session setup and delivered when the shell opens,
so a configuration change made by the same `ssh` invocation that opens the
shell is reported. Sessions that never open a shell (`sftp`, or the `can`
subsystem alone) get no messages.

# UART pins

UART RX/TX pins are defined per-board in `boards/*.toml` files inside the
`ssh-stamp-esp32-boards` crate. Each board feature (e.g.
`board-esp32c6-devkitc`) selects a specific PCB and its pin assignments.
The TOML files are the single source of truth — no other file in the
repository hard-codes UART pin numbers.

To see the available boards and their pin assignments, run:

```
cargo build-doc
```

Then open `target/riscv32imac-unknown-none-elf/doc/ssh_stamp_esp32_boards/index.html`,
which contains the auto-generated per-board pin assignment table.