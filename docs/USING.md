<!--
SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>

SPDX-License-Identifier: GPL-3.0-or-later
-->

## First boot & provisioning

Once the flash process finishes successfully, follow the steps below:

1. On first boot the device generates a random WPA2 PSK and prints it to the serial console, together with the SSID, MAC, SSH hostkey fingerprint and, once the network is up, the AP's IP address (see [On the serial console](#on-the-serial-console)):

```json
{
  "schema": 1,
  "event": "boot",
  "ident": "SSH-2.0-Sunset-0.6.0-ssh-stamp-1.0.4",
  "wifi_ap": {
    "ssid": "<SSID>",
    "psk": "<PSK>",
    "band": "2.4GHz",
    "channel": 1
  },
  "mac": "<MAC>",
  "hostkey_fingerprint": "<FINGERPRINT>",
  "first_login": true
}
{
  "schema": 1,
  "event": "net_up",
  "role": "ap",
  "ssid": "<SSID>",
  "ip": "192.168.4.1"
}
```

2. Connect a laptop/phone to the WiFi AP using the printed SSID and PSK, then SSH into the device at `root@192.168.4.1`.

   The session is a pipe to the target's UART. As with a shell, Ctrl-D ends the session and Ctrl-C is sent to the target, so it can interrupt a program running there. To send Ctrl-D to the target as well, make the session fully transparent:

   ```
   export SSH_STAMP_TRANSPARENT=1
   ssh -o SendEnv=SSH_STAMP_TRANSPARENT root@192.168.4.1
   ```

   and leave it with Enter, then `~.` (OpenSSH's escape sequence). Sessions without a terminal, such as `ssh root@192.168.4.1 < file`, are always transparent, so binary data passes through unchanged.

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

- To set the UART bridge line parameters (defaults to 115200 8N1):
```
export SSH_STAMP_UART_BAUD="921600"
export SSH_STAMP_UART_DATA_BITS="8"
export SSH_STAMP_UART_PARITY="none"
export SSH_STAMP_UART_STOP_BITS="1"
ssh -o SendEnv='SSH_STAMP_UART_*' root@192.168.4.1
```
Accepted values: baud `300`-`5000000`, data bits `5`-`8`, parity `none`/`even`/`odd`, stop bits `1`/`2`. Each is persisted independently, so only the ones you send change.

Notes:
- `SSH_STAMP_PUBKEY` is accepted on first-boot to add the initial admin key.
- `SSH_STAMP_WIFI_AP_SSID` and `SSH_STAMP_WIFI_AP_PSK` may be applied while authenticated via pubkey (or on first-boot). After a successful change the device persists the settings and performs a software reset so the new WiFi settings take effect.
- `SSH_STAMP_WIFI_BAND` selects the AP radio band. Only the ESP32-C5 supports 5GHz; other chips ignore the setting and stay on 2.4GHz.
- `SSH_STAMP_UART_*` is supported on every target. The bridge configures its UART once at boot, so the device also resets here: the new line settings are live on the next connection.
- If you prefer a single-step provisioning, export all three env vars locally and forward them with `SendEnv` in the same SSH invocation.

If your SSH client doesn't forward environment variables by default, use the `-o SendEnv=VAR` option as shown above or configure `SendEnv` in your SSH client config.

# Machine-readable output

The device reports on its serial console as JSON, pretty-printed so it reads well on a terminal
and still parses with `jq`. Every object opens with `{` and closes with `}`
alone on their own lines, and its first two keys are always the same:

```json
{
  "schema": 1,
  "event": "boot",
  ...
}
```

`schema` is the version of this JSON format. It only changes if the meaning of existing
fields changes — new fields get added without bumping it, so **ignore keys you
do not recognise** and your parser will keep working across upgrades.

## On the serial console

The console is shared with the ESP32 ROM bootloader and the second-stage
bootloader, neither of which is JSON. Cut the objects out by their framing and
hand them to `jq`:

```
espflash monitor | sed -un '/^{$/,/^}$/p' | jq -c --unbuffered .
```

Two objects are emitted at startup. `boot` carries the provisioning details,
shown in [First boot](#first-boot--provisioning) above, and `net_up` follows
once the network stack has an address, which is not known at boot. After
that, `station_connected` / `station_disconnected` report clients joining the
AP (`mac`, `aid`, and a `reason` on disconnect), and in station mode
`wifi_connected` / `wifi_disconnected` report the upstream AP (`ssid`,
`bssid`, `channel` or `reason`).

Everything else the firmware logs arrives as a `log` event:

```json
{
  "schema": 1,
  "event": "log",
  "level": "info",
  "target": "ssh_stamp_esp32::network::wifi",
  "msg": "Wifi configuring Access Point Mode"
}
```

`ESP_LOG` (set in `.cargo/config.toml`) chooses which levels are printed, as a
global level plus optional `target=level` overrides, e.g.
`info,esp_radio=warn`. On the ESP32-C2, C3 and S2, which lack the atomics a
custom logger needs, `log` events stay plain `INFO - message` lines; every
other object above is JSON on all chips.

So a first-boot provisioning script can pick up what it needs directly:

```
PSK=$(espflash monitor | sed -un '/^{$/,/^}$/p' \
        | jq -r --unbuffered 'select(.event=="boot") | .wifi_ap.psk' | head -1)
```

Note `boot` **does** contain the WPA2 PSK in the clear. It has to: nothing can
associate with the AP without it, and it is generated on first boot and
printed nowhere else. This is a local serial cable rather than a network peer
— but it does mean a captured console log is a credential.

# Pin assignments

UART, CAN and I2C pins are defined per-board in `boards/*.toml` files inside
the board support crate of each platform. For Espressif boards that is
[`boards/ssh-stamp-esp/ssh-stamp-esp32-boards/boards/`](../boards/ssh-stamp-esp/ssh-stamp-esp32-boards/boards). Each board feature (e.g. `board-esp32c6-devkitc`) selects a
specific PCB and its pin assignments. The TOML files are the single source of
truth — no other file in the repository hard-codes pin numbers.

To see which GPIO each bus uses on each board, and which buses a board does
not support yet, read the board catalog on
[docs.rs](https://docs.rs/ssh-stamp-esp32-boards) or build the documentation
locally:

```
cargo xtask esp32c6-devkitc doc --no-deps --lib --workspace --exclude xtask
```

Then open the board support crate's front page,
`target/boards/esp32c6-devkitc/riscv32imac-unknown-none-elf/doc/ssh_stamp_esp32_boards/index.html`.
Its catalog table is regenerated from the TOML files on every run.

(`cargo xtask list` gives the board list as a quick terminal summary, without
the pins.)