#!/usr/bin/env bash
set -euo pipefail

SIDECARS=(sprout-acp sprout-mcp-server sprout-agent sprout-dev-mcp git-credential-nostr)
TARGET=${1:-$(rustc -vV | sed -n 's|host: ||p')}
BINARIES_DIR="desktop/src-tauri/binaries"

# sprout-cli produces a binary named "sprout" but Tauri rejects a sidecar
# with the same name as the Cargo package, so we bundle it as "sprout-cli".
RENAMES=("sprout:sprout-cli")

missing=()
for bin in "${SIDECARS[@]}"; do
    [[ -f "target/release/$bin" ]] || missing+=("$bin")
done
for mapping in "${RENAMES[@]}"; do
    src="${mapping%%:*}"
    [[ -f "target/release/$src" ]] || missing+=("$src")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    echo "Error: missing release binaries: ${missing[*]}" >&2
    echo "Run 'cargo build --release -p sprout-acp -p sprout-mcp -p sprout-agent -p sprout-dev-mcp -p git-credential-nostr -p sprout-cli' first." >&2
    exit 1
fi

mkdir -p "$BINARIES_DIR"
for bin in "${SIDECARS[@]}"; do
    cp "target/release/$bin" "$BINARIES_DIR/${bin}-${TARGET}"
done
for mapping in "${RENAMES[@]}"; do
    src="${mapping%%:*}"
    dst="${mapping##*:}"
    cp "target/release/$src" "$BINARIES_DIR/${dst}-${TARGET}"
done
echo "Sidecars bundled for $TARGET"
