#!/usr/bin/env bash
set -euo pipefail

BY_ID_DIR="/dev/serial/by-id"

usage() {
  cat <<'EOF'
Usage:
  link_serials.sh <MAP_FILE> <OUT_DIR>

Arguments:
  MAP_FILE   Path to mapping file (format: serial_id=function_name)
  OUT_DIR    Directory where function symlinks will be created

Example:
  sudo ./scripts/link_serials.sh config/serial_map.conf /dev/serial/by-function
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -ne 2 ]]; then
  usage
  exit 1
fi

MAP_FILE="$1"
OUT_DIR="$2"

if [[ ! -f "$MAP_FILE" ]]; then
  echo "error: MAP_FILE not found: $MAP_FILE" >&2
  exit 1
fi

# Cleaning condition at the beginning:
# remove existing symlinks in OUT_DIR so the directory is rebuilt from MAP_FILE
mkdir -p "$OUT_DIR"
find "$OUT_DIR" -mindepth 1 -maxdepth 1 -type l -delete

while IFS='=' read -r serial_id function_name; do
  # skip blanks/comments/invalid lines
  [[ -z "${serial_id:-}" ]] && continue
  [[ "${serial_id:0:1}" == "#" ]] && continue
  [[ -z "${function_name:-}" ]] && continue

  src="$BY_ID_DIR/$serial_id"
  dst="$OUT_DIR/$function_name"

  if [[ -e "$src" ]]; then
    ln -sfn "$src" "$dst"
    echo "linked $dst -> $src"
  else
    echo "missing: $src" >&2
  fi
done < "$MAP_FILE"
