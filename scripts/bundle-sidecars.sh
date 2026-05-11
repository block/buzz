#!/usr/bin/env bash
set -euo pipefail

SIDECARS=(sprout-acp sprout-mcp-server sprout-agent sprout-dev-mcp git-credential-nostr)
TARGET=${1:-$(rustc -vV | sed -n 's|host: ||p')}
BINARIES_DIR="desktop/src-tauri/binaries"

missing=()
for bin in "${SIDECARS[@]}"; do
    [[ -f "target/release/$bin" ]] || missing+=("$bin")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    echo "Error: missing release binaries: ${missing[*]}" >&2
    echo "Run 'cargo build --release -p sprout-acp -p sprout-mcp -p sprout-agent -p sprout-dev-mcp -p git-credential-nostr' first." >&2
    exit 1
fi

mkdir -p "$BINARIES_DIR"
for bin in "${SIDECARS[@]}"; do
    cp "target/release/$bin" "$BINARIES_DIR/${bin}-${TARGET}"
done
echo "Sidecars bundled for $TARGET"
