#!/usr/bin/env bash
set -euo pipefail

BIN="/usr/local/bin/link_serials_by_function.sh"
MAP="/etc/serials_map.conf"
SERVICE="/etc/systemd/system/link-serials-by-function.service"
RULES="/etc/udev/rules.d/99-link-serials-by-function.rules"
TMPFILES="/etc/tmpfiles.d/serials-by-function.conf"
OUT_DIR="/dev/serial/by-function"
REMOVE_MAP="false"

usage() {
  cat <<'EOF'
Usage:
  sudo ./uninstall_serial_by_function.sh [--out-dir PATH] [--remove-map]

Options:
  --out-dir PATH   Symlink directory to clean/remove (default: /dev/serial/by-function)
  --remove-map     Also remove /etc/serials_map.conf
  -h, --help       Show help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --remove-map) REMOVE_MAP="true"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "error: unknown option: $1" >&2; usage; exit 1 ;;
  esac
done

if [[ $EUID -ne 0 ]]; then
  echo "error: run as root (sudo)." >&2
  exit 1
fi

# Stop service if present
if systemctl list-unit-files | grep -q '^link-serials-by-function.service'; then
  systemctl stop link-serials.service || true
fi

# Remove installed files
rm -f "$SERVICE" "$RULES" "$TMPFILES" "$BIN"

if [[ "$REMOVE_MAP" == "true" ]]; then
  rm -f "$MAP"
fi

# Clean runtime symlinks and directory
if [[ -d "$OUT_DIR" ]]; then
  find "$OUT_DIR" -mindepth 1 -maxdepth 1 -type l -delete || true
  rmdir "$OUT_DIR" 2>/dev/null || true
fi

# Reload managers
systemctl daemon-reload
udevadm control --reload-rules
systemd-tmpfiles --remove "$TMPFILES" 2>/dev/null || true

echo "Uninstalled serial by function."
