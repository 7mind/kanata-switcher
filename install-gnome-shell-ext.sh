#!/usr/bin/env bash
set -xeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT_DIR="$SCRIPT_DIR/src/gnome-extension"
EXT_UUID="kanata-switcher@7mind.io"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

cp -R "$EXT_DIR"/. "$TMP_DIR"
glib-compile-schemas "$TMP_DIR/schemas"

gnome-extensions pack "$TMP_DIR" --force --out-dir=/tmp
gnome-extensions install "/tmp/${EXT_UUID}.shell-extension.zip" --force

set +x

echo ""
echo "Extension installed. Restart GNOME Shell, then run:"
echo "  gnome-extensions enable $EXT_UUID"
