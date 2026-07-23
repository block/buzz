#!/usr/bin/env bash
# Install buzz-backend-crabbox onto PATH so Buzz Desktop can discover it.
#
# Desktop scans PATH (plus ~/.local/bin and the app bundle MacOS dir) for
# executables named buzz-backend-*. After install, restart Desktop and pick
# "crabbox" under Run on when creating/starting an agent.
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
src="$root/buzz-backend-crabbox"
dest_dir="${BUZZ_BACKEND_BIN_DIR:-$HOME/.local/bin}"
dest="$dest_dir/buzz-backend-crabbox"

if [[ ! -f "$src" ]]; then
  echo "install: missing $src" >&2
  exit 1
fi

mkdir -p "$dest_dir"
chmod +x "$src"

if [[ -e "$dest" || -L "$dest" ]]; then
  rm -f "$dest"
fi

ln -s "$src" "$dest"
echo "installed: $dest -> $src"
echo
echo "Next:"
echo "  1. brew install openclaw/tap/crabbox && crabbox login --url <broker>"
echo "  2. ensure buzz-acp (and ideally buzz) are on PATH"
echo "  3. restart Buzz Desktop and choose Run on → crabbox"
