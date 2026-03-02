#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  sudo ./install_serials_by_function.sh <MAP_FILE> [OUT_DIR]

Arguments:
  MAP_FILE   Source mapping file (serial_id=function_name)
  OUT_DIR    Target symlink directory (default: /dev/serial/by-function)
EOF
}

[[ "${1:-}" =~ ^(-h|--help)$ ]] && { usage; exit 0; }
[[ $# -lt 1 || $# -gt 2 ]] && { usage; exit 1; }

if [[ $EUID -ne 0 ]]; then
  echo "error: run as root (sudo)." >&2
  exit 1
fi

MAP_SRC="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
OUT_DIR="${2:-/dev/serial/by-function}"

if [[ ! -f "$MAP_SRC" ]]; then
  echo "error: map file not found: $MAP_SRC" >&2
  exit 1
fi

# Install locations
BIN="/usr/local/bin/link_serials_by_function.sh"
MAP_DST="/etc/serials_map.conf"
SERVICE="/etc/systemd/system/link-serials-by-function.service"
RULES="/etc/udev/rules.d/99-link-serials-by-function.rules"
TMPFILES="/etc/tmpfiles.d/serials-by-function.conf"

# Expect your existing linker script in repo
if [[ ! -f "scripts/link_serials.sh" ]]; then
  echo "error: scripts/link_serials.sh not found in current directory." >&2
  exit 1
fi

install -m 0755 scripts/link_serials.sh "$BIN"
install -m 0644 "$MAP_SRC" "$MAP_DST"

cat > "$SERVICE" <<EOF
[Unit]
Description=Refresh serial function symlinks

[Service]
Type=oneshot
ExecStart=$BIN $MAP_DST $OUT_DIR
EOF

cat > "$RULES" <<'EOF'
ACTION=="add|remove", SUBSYSTEM=="tty", KERNEL=="ttyUSB[0-9]*", TAG+="systemd", ENV{SYSTEMD_WANTS}+="link-serials.service"
ACTION=="add|remove", SUBSYSTEM=="tty", KERNEL=="ttyACM[0-9]*", TAG+="systemd", ENV{SYSTEMD_WANTS}+="link-serials.service"
EOF

cat > "$TMPFILES" <<EOF
d $OUT_DIR 0755 root root -
EOF

systemctl daemon-reload
udevadm control --reload-rules
systemd-tmpfiles --create "$TMPFILES"

# Initial run now
systemctl start link-serials-by-function.service

echo "Installed."
echo "Map:      $MAP_DST"
echo "Out dir:  $OUT_DIR"
echo "Service:  link-serials-by-function.service"
