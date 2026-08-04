#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
key_script="scripts/desktop-release-cache-key.py"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cp -R "$repo_root"/. "$tmp/repo"
cd "$tmp/repo"

args=(--platform Linux --target x86_64-unknown-linux-gnu --features mesh-llm --native-inputs ubuntu-24.04-mold)
original=$("$key_script" "${args[@]}")
python3 - <<'PY'
from pathlib import Path
manifest = Path("desktop/src-tauri/Cargo.toml")
manifest.write_text(manifest.read_text().replace('version = "0.5.4"', 'version = "9.8.7"', 1))
lock = Path("desktop/src-tauri/Cargo.lock")
text = lock.read_text()
start = text.index('name = "buzz-desktop"')
version = text.index('version = "0.5.4"', start)
lock.write_text(text[:version] + 'version = "9.8.7"' + text[version + len('version = "0.5.4"'):])
PY
version_only=$("$key_script" "${args[@]}")
[[ "$original" == "$version_only" ]] || { echo "desktop version changed cache key" >&2; exit 1; }
printf '\n# dependency input\n' >> crates/buzz-acp/Cargo.toml
dependency_changed=$("$key_script" "${args[@]}")
[[ "$original" != "$dependency_changed" ]] || { echo "dependency manifest did not change cache key" >&2; exit 1; }
[[ "$original" == desktop-rust-release-v1-Linux-x86_64-unknown-linux-gnu-* ]] || { echo "unexpected key: $original" >&2; exit 1; }
echo "desktop release cache key contract passed"
