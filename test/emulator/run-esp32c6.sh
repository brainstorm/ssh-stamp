#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
#
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Boot ssh-stamp on an emulated ESP32-C6 and expose its SSH port on the host.
#
# No hardware required. Builds the firmware, merges it into a flash image with
# espflash, and runs it under esp-emu (Espressif's RISC-V emulator).
#
#   ./test/emulator/run-esp32c6.sh                 # boot, print serial output
#   ./test/emulator/run-esp32c6.sh --ssh-port 2222 # also forward SSH to :2222
#
# Requires: esp-emu (https://github.com/espressif/esp-emulator), espflash.
#
# NOTE on networking: the emulator plays the *access point*, so SSH is only
# reachable when the firmware runs in station mode and joins it. A factory
# ssh-stamp boots as an AP instead, and nothing on the host can associate with
# that, so --ssh-port only does something useful once the config carries
# station credentials matching --wifi-ssid/--wifi-password below. See
# docs/EMULATION.md.

set -euo pipefail

CHIP=esp32c6
BOARD=esp32c6-devkitc
TARGET=riscv32imac-unknown-none-elf
WIFI_SSID=myssid
WIFI_PASS=mypassword
GUEST_IP=192.168.4.2   # DHCP lease the emulator hands the firmware
TIMEOUT=300
SSH_PORT=""

while [ $# -gt 0 ]; do
    case "$1" in
        --ssh-port) SSH_PORT="$2"; shift 2 ;;
        --timeout)  TIMEOUT="$2";  shift 2 ;;
        --ssid)     WIFI_SSID="$2"; shift 2 ;;
        --password) WIFI_PASS="$2"; shift 2 ;;
        -h|--help)  sed -n '6,22p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

for tool in esp-emu espflash cargo; do
    command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

elf="target/boards/$BOARD/$TARGET/release/ssh-stamp-esp32"
image="$(mktemp -t ssh-stamp-XXXXXX.bin)"
trap 'rm -f "$image"' EXIT

echo ">> building $BOARD"
cargo xtask "$BOARD" build --release

# esp-emu wants one merged flash image (bootloader + partition table + app),
# which espflash can produce directly from the Rust ELF — no ESP-IDF needed.
echo ">> merging flash image"
espflash save-image --chip "$CHIP" --merge \
    --partition-table ssh-stamp-esp32/partitions.csv "$elf" "$image"

net="user"
if [ -n "$SSH_PORT" ]; then
    net="user,hostfwd=tcp::${SSH_PORT}-${GUEST_IP}:22"
    echo ">> forwarding host :$SSH_PORT to guest $GUEST_IP:22"
fi

echo ">> booting (timeout ${TIMEOUT}s, Ctrl-C to stop)"
exec esp-emu --chip "$CHIP" \
    --firmware "$image" \
    --elf "$elf" \
    --wifi-ssid "$WIFI_SSID" \
    --wifi-password "$WIFI_PASS" \
    --net "$net" \
    --timeout "$TIMEOUT"
