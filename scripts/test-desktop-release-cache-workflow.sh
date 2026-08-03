#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
release="$root/.github/workflows/release.yml"
proof="$root/.github/workflows/desktop-release-cache-proof.yml"
canaries=(
  "$root/.github/workflows/signed-macos-canary.yml"
  "$root/.github/workflows/windows-canary.yml"
  "$root/.github/workflows/linux-canary.yml"
)

if grep -q 'desktop-rust-release-v1\|desktop-release-cache-key' "$release"; then
  echo "Gate 1 must not alter the release cache path" >&2
  exit 1
fi
for workflow in "${canaries[@]}"; do
  [[ $(grep -c 'actions/cache/restore@' "$workflow") -ge 1 ]]
  [[ $(grep -c 'actions/cache/save@' "$workflow") -ge 1 ]]
  grep -q 'steps.rust_cache.outputs.cache-hit' "$workflow"
  grep -q '!desktop/src-tauri/target/\*\*/release/bundle' "$workflow"
  if grep -q 'restore-keys:.*desktop-rust\|Swatinem/rust-cache' "$workflow"; then
    echo "release Cargo cache must use split actions with no fallback: $workflow" >&2
    exit 1
  fi
done
[[ $(grep -c 'actions/cache/save@' "$proof") -eq 0 ]]
grep -q 'refs/tags/cache-proof-' "$proof"
grep -q 'CACHE_HIT.*steps.rust_cache.outputs.cache-hit' "$proof"
grep -q 'CACHE_KEY.*steps.rust_cache.outputs.cache-primary-key' "$proof"
echo "desktop release cache workflow contract passed"
