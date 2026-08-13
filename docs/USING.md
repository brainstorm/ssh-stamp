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

# Machine-readable output

Everything below is also emitted as JSON, one object per line, for scripting
provisioning instead of scraping prose. Every object starts with the same
marker:

```text
{"ssh_stamp":1,"event":"boot", ...}
```

`ssh_stamp` is the schema version. It only changes if the meaning of existing
fields changes — new fields get added without bumping it, so **ignore keys you
do not recognise** and your parser will keep working across upgrades.

## On the serial console

The console is shared with the ESP32 ROM bootloader, the second-stage
bootloader and the ordinary log, none of which is JSON, and the log adds an
`INFO - ` prefix in front of our line. Select the JSON with the marker and
strip the prefix in one step:

```
espflash monitor | grep --line-buffered -ao '{"ssh_stamp".*}' | jq -c .
```

`-a` treats the stream as text (the ROM prelude contains binary at the wrong
baud rate), `-o` prints only the matched JSON rather than the whole line, and
`--line-buffered` stops grep holding output back while you wait.

Two objects are emitted at startup. `boot` carries the provisioning details:

```json
{"ssh_stamp":1,"event":"boot","wifi_ap":{"ssid":"ssh-stamp-a1b2","psk":"hunter2hunter2","band":"2.4GHz"},"mac":"40:4c:ca:12:34:56","hostkey_fingerprint":"SHA256:abcdef","first_login":true}
```

and `net_up` follows once the network stack has an address, which is not known
at boot:

```json
{"ssh_stamp":1,"event":"net_up","role":"ap","ssid":"ssh-stamp-a1b2","ip":"192.168.4.1"}
```

So a first-boot provisioning script can pick up what it needs directly:

```
PSK=$(espflash monitor | grep -ao '{"ssh_stamp".*}' \
        | jq -r --unbuffered 'select(.event=="boot") | .wifi_ap.psk' | head -1)
```

Note `boot` **does** contain the WPA2 PSK in the clear. It has to: nothing can
associate with the AP without it, and it is generated on first boot and
printed nowhere else. This is a local serial cable rather than a network peer
— but it does mean a captured console log is a credential.

## Over SSH

Set `SSH_STAMP_NOTICES=json` and the session messages become JSON on stderr
instead of prose:

```
export SSH_STAMP_NOTICES=json
ssh -o SendEnv=SSH_STAMP_NOTICES root@192.168.4.1 2>&1 >/dev/null | jq -c .
```

Note the `2>&1 >/dev/null` ordering: it sends stderr to the pipe and discards
stdout, so `jq` sees the messages and not the target's UART traffic.

**Send `SSH_STAMP_NOTICES` before any other `SSH_STAMP_*` variable.** The
switch discards anything already queued — otherwise prose from before the
switch would be interleaved into the JSON stream and break the parse — so a
change made by a variable sent earlier goes unreported:

```
# Right: the SSID change is reported as JSON
ssh -o SendEnv=SSH_STAMP_NOTICES -o SendEnv=SSH_STAMP_WIFI_AP_SSID root@192.168.4.1
# Wrong: the SSID change is applied, but silently
ssh -o SendEnv=SSH_STAMP_WIFI_AP_SSID -o SendEnv=SSH_STAMP_NOTICES root@192.168.4.1
```

```json
{"ssh_stamp":1,"event":"config_changed","key":"wifi_ap_ssid","from":"ssh-stamp-a1b2","to":"SshStampSSID"}
{"ssh_stamp":1,"event":"config_secret_changed","key":"wifi_ap_psk","len":24}
{"ssh_stamp":1,"event":"config_saved","text":"config: saved to flash"}
{"ssh_stamp":1,"event":"config","uart":{"rx":17,"tx":16},"wifi_ap":{"ssid":"SshStampSSID","psk_set":true,"band":"2.4GHz"},"wifi_sta":null,"mac":"40:4c:ca:12:34:56","mac_random":false,"ipv4":{"dhcp":true},"authorised_keys":{"used":1,"slots":1},"first_login":false}
{"ssh_stamp":1,"event":"bridge","text":"bridge connected"}
```

The configuration arrives as one `config` object rather than a line per field,
so it can be indexed directly:

```
$ ... | jq -r 'select(.event=="config") | .wifi_ap.ssid'
SshStampSSID
```

Did a provisioning change actually take?

```
$ ... | jq -r 'select(.event=="config_changed") | "\(.key): \(.from) -> \(.to)"'
wifi_ap_ssid: ssh-stamp-a1b2 -> SshStampSSID
```

Why was one rejected?

```
$ ... | jq -r 'select(.event=="rejected") | "\(.what): \(.reason)"'
SSH_STAMP_WIFI_BAND: must be 2.4g, 5g or auto
```

Two things are deliberately not in the JSON:

- **Secrets.** Over SSH a changed password reports only
  `config_secret_changed` with its length, and the summary reports
  `"psk_set":true`. The console `boot` object is the sole exception, for the
  reason above.
- **The pre-authentication banner.** `SSH_STAMP_NOTICES` is an environment
  variable, and those arrive after authentication, so the device cannot know
  you wanted JSON at the point it sends the banner. It is always prose.

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
cargo xtask doc
```

(`cargo xtask list` gives the same board list as a quick terminal summary.)

Then open `target/riscv32imac-unknown-none-elf/doc/ssh_stamp_esp32_boards/index.html`,
which contains the auto-generated per-board pin assignment table.