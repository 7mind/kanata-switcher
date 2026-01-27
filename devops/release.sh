#!/usr/bin/env bash
set -euxo pipefail
cat "$0"
read -p "Press Enter to run, Ctrl+C to abort..."

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "${REPO_ROOT}"

read -r -p "New version: " NEW_VERSION
if [[ -z "${NEW_VERSION}" ]]; then
  echo "New version is required" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Git worktree is not clean." >&2
  read -r -p "Continue anyway? [y/N] " CONFIRM_DIRTY
  if [[ "${CONFIRM_DIRTY}" != "y" && "${CONFIRM_DIRTY}" != "Y" ]]; then
    echo "Aborting due to dirty worktree." >&2
    exit 1
  fi
fi

export NEW_VERSION
python - <<'PY'
from pathlib import Path
import os

new_version = os.environ.get("NEW_VERSION")
if not new_version:
    raise SystemExit("NEW_VERSION env var not set")

path = Path("Cargo.toml")
lines = path.read_text(encoding="utf-8").splitlines()

in_package = False
updated = False
new_lines = []
for line in lines:
    if line.strip() == "[package]":
        in_package = True
        new_lines.append(line)
        continue
    if in_package and line.startswith("[") and line.strip().endswith("]"):
        in_package = False
    if in_package and line.strip().startswith("version = ") and not updated:
        new_lines.append(f'version = "{new_version}"')
        updated = True
        continue
    new_lines.append(line)

if not updated:
    raise SystemExit("Failed to update version in [package]")

path.write_text("\n".join(new_lines) + "\n", encoding="utf-8")
PY

if ! nix develop "${REPO_ROOT}" --command cargo generate-lockfile; then
  echo "Failed to regenerate Cargo.lock" >&2
  exit 1
fi

git add Cargo.toml Cargo.lock

git commit -m "Release ${NEW_VERSION}"

git tag "v${NEW_VERSION}"
