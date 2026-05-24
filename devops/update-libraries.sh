#!/usr/bin/env bash
set -euxo pipefail
cat "$0"
read -p "Press Enter to run, Ctrl+C to abort..."

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "${REPO_ROOT}"

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Git worktree is not clean." >&2
  read -r -p "Continue anyway? [y/N] " CONFIRM_DIRTY
  if [[ "${CONFIRM_DIRTY}" != "y" && "${CONFIRM_DIRTY}" != "Y" ]]; then
    echo "Aborting due to dirty worktree." >&2
    exit 1
  fi
fi

if ! nix develop "${REPO_ROOT}" --command cargo metadata --locked --format-version 1 --no-deps >/dev/null; then
  echo "Cargo.toml and Cargo.lock must be synchronized before updating libraries." >&2
  echo "Run devops/release.sh for kanata-switcher version changes first." >&2
  exit 1
fi

if ! nix develop "${REPO_ROOT}" --command cargo update; then
  echo "Failed to update Cargo.lock dependencies" >&2
  exit 1
fi

if git diff --quiet -- Cargo.lock; then
  echo "Cargo.lock is already up to date."
  exit 0
fi

git add Cargo.lock

git commit -m "Update Cargo dependencies"
