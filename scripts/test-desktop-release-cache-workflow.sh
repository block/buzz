#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
release="$root/.github/workflows/release.yml"
proof="$root/.github/workflows/desktop-release-cache-proof.yml"
canaries=(
  "$root/.github/workflows/signed-macos-canary.yml"
  "$root/.github/workflows/macos-intel-canary.yml"
  "$root/.github/workflows/windows-canary.yml"
  "$root/.github/workflows/linux-canary.yml"
)

if grep -q 'desktop-rust-release-v1\|desktop-release-cache-key' "$release"; then
  echo "Gate 1 must not alter the release cache path" >&2
  exit 1
fi
for workflow in "${canaries[@]}"; do
  grep -q 'refs/heads/main' "$workflow"
  grep -q 'desktop-native-toolchain-id.sh' "$workflow"
  grep -q 'steps.native_toolchain.outputs.id' "$workflow"
  grep -q 'actions/cache/restore@' "$workflow"
  grep -q 'actions/cache/save@' "$workflow"
  grep -q 'steps.rust_cache.outputs.cache-hit' "$workflow"
  grep -q '!desktop/src-tauri/target/\*\*/release/bundle' "$workflow"
  if grep -q 'restore-keys:.*desktop-rust\|Swatinem/rust-cache' "$workflow"; then
    echo "release Cargo cache must use split actions with no fallback: $workflow" >&2
    exit 1
  fi
done

# GitHub expressions must enter cache-key steps through env, never by direct
# interpolation into generated shell scripts. This blocks shell injection if a
# matrix or upstream output ever becomes attacker-controlled.
python3 - "$proof" "${canaries[@]}" <<'PY'
import pathlib
import re
import sys

for filename in sys.argv[1:]:
    text = pathlib.Path(filename).read_text()
    steps = re.findall(
        r"(?ms)^      - name: Compute exact release cache key\n(.*?)(?=^      - (?:name:|uses:)|\Z)",
        text,
    )
    if not steps:
        raise SystemExit(f"cache-key step missing: {filename}")
    for step in steps:
        run = re.search(r"(?ms)^        run: \|\n(.*?)(?=^        \S|\Z)", step)
        if not run:
            raise SystemExit(f"cache-key run block missing: {filename}")
        if "${{" in run.group(1):
            raise SystemExit(f"GitHub expression interpolated into cache-key shell: {filename}")
        if "NATIVE_TOOLCHAIN_ID: ${{ steps.native_toolchain.outputs.id }}" not in step:
            raise SystemExit(f"native toolchain output not passed through env: {filename}")
PY

# Producer/proof coverage must match all four release targets and features.
for target in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu x86_64-pc-windows-msvc; do
  grep -q -- "$target" "$proof" || { echo "proof missing $target" >&2; exit 1; }
done
grep -q -- '--features mesh-llm' "$proof"
grep -q -- '--features default' "$proof"
[[ $(grep -c 'actions/cache/save@' "$proof") -eq 0 ]]
[[ $(grep -c 'Require exact cache hit' "$proof") -eq 3 ]]
grep -q 'refs/tags/cache-proof-' "$proof"

# Linux producer must match release's default linker, not the CI-only mold path.
if grep -q 'setup-mold\|ubuntu-24.04-mold' "$root/.github/workflows/linux-canary.yml"; then
  echo "Linux cache producer diverges from the release linker" >&2
  exit 1
fi
if grep -q 'setup-mold' "$release"; then
  echo "release linker changed; re-review cache equivalence" >&2
  exit 1
fi

echo "desktop release cache workflow contract passed"
