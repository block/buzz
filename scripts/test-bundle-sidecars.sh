#!/usr/bin/env bash
# test-bundle-sidecars.sh — Validate scripts/bundle-sidecars.sh fail-closed behavior
#
# Tests:
#  1. Fails when release binaries are missing from target directory.
#  2. Fails when release binaries exist but are 0-byte empty placeholders.
#  3. Succeeds and sets mode 755 when release binaries are non-empty executables.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

TARGET="test-target-triple"
mkdir -p "$tmp/scripts" "$tmp/target/$TARGET/release" "$tmp/desktop/src-tauri/binaries"
cp "$repo_root/scripts/bundle-sidecars.sh" "$tmp/scripts/bundle-sidecars.sh"
chmod +x "$tmp/scripts/bundle-sidecars.sh"

SIDECARS=(buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz buzz-backend-kubernetes)

echo "==> Case 1: Missing release binaries must fail closed"
set +e
output=$(cd "$tmp" && ./scripts/bundle-sidecars.sh "$TARGET" 2>&1)
rc=$?
set -e
if [[ $rc -eq 0 ]]; then
    echo "FAIL: bundle-sidecars.sh succeeded when release binaries were missing" >&2
    exit 1
fi
if [[ "$output" != *"missing or empty release binaries"* ]]; then
    echo "FAIL: unexpected error message on missing binaries: $output" >&2
    exit 1
fi
echo "PASS: missing release binaries rejected"

echo "==> Case 2: 0-byte placeholder binaries must fail closed"
for bin in "${SIDECARS[@]}"; do
    touch "$tmp/target/$TARGET/release/$bin"
done
set +e
output=$(cd "$tmp" && ./scripts/bundle-sidecars.sh "$TARGET" 2>&1)
rc=$?
set -e
if [[ $rc -eq 0 ]]; then
    echo "FAIL: bundle-sidecars.sh succeeded with 0-byte placeholder binaries" >&2
    exit 1
fi
if [[ "$output" != *"missing or empty release binaries"* ]]; then
    echo "FAIL: unexpected error message on 0-byte binaries: $output" >&2
    exit 1
fi
echo "PASS: 0-byte placeholder binaries rejected"

echo "==> Case 3: Real non-empty binaries succeed and are staged with executable permissions"
for bin in "${SIDECARS[@]}"; do
    printf '#!/bin/sh\necho "%s"\n' "$bin" > "$tmp/target/$TARGET/release/$bin"
    chmod +x "$tmp/target/$TARGET/release/$bin"
done

(cd "$tmp" && ./scripts/bundle-sidecars.sh "$TARGET")

for bin in "${SIDECARS[@]}"; do
    staged="$tmp/desktop/src-tauri/binaries/${bin}-${TARGET}"
    if [[ ! -s "$staged" ]]; then
        echo "FAIL: staged binary $staged is missing or empty" >&2
        exit 1
    fi
    if [[ ! -x "$staged" ]]; then
        echo "FAIL: staged binary $staged is not executable" >&2
        exit 1
    fi
done
echo "PASS: valid binaries staged and executable"

echo "==> Case 4: --print-cargo-packages matches the staged sidecar set"
packages=$(cd "$tmp" && ./scripts/bundle-sidecars.sh --print-cargo-packages "$TARGET")
expected="-p buzz-acp -p buzz-agent -p buzz-dev-mcp -p git-credential-nostr -p buzz-cli -p buzz-backend-kubernetes"
if [[ "$packages" != "$expected" ]]; then
    echo "FAIL: unexpected cargo packages for $TARGET: $packages" >&2
    exit 1
fi
packages=$(cd "$tmp" && ./scripts/bundle-sidecars.sh --print-cargo-packages x86_64-pc-windows-msvc)
if [[ "$packages" == *buzz-backend-kubernetes* ]]; then
    echo "FAIL: Windows build must not include buzz-backend-kubernetes: $packages" >&2
    exit 1
fi
echo "PASS: cargo package list derived from the sidecar list"

echo "All bundle-sidecars.sh tests passed."
