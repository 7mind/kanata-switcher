#!/usr/bin/env bash
set -xeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT_DIR="$SCRIPT_DIR/src/gnome-extension"
EXT_UUID="kanata-switcher@7mind.io"

gnome-extensions pack "$EXT_DIR" --force --out-dir=/tmp
gnome-extensions install "/tmp/${EXT_UUID}.shell-extension.zip" --force

set +x

echo ""
echo "Extension installed. Restart GNOME Shell, then run:"
echo "  gnome-extensions enable $EXT_UUID"
