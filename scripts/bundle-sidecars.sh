#!/usr/bin/env bash
set -euo pipefail

SIDECARS=(buzz-acp buzz-dev-mcp git-credential-nostr buzz)
# goose-acp lives outside the workspace (own lockfile — see goose-acp/Cargo.toml)
# so it builds into its own target dir and is staged separately below.
GOOSE_ACP_DIR="goose-acp"
HOST=$(rustc -vV | sed -n 's|host: ||p')
TARGET=${1:-$HOST}
BINARIES_DIR="desktop/src-tauri/binaries"

# When --target is passed explicitly to cargo (even if it matches the host),
# binaries land in target/<triple>/release/. Without --target, they land in
# target/release/. The script receives the target as $1 only when cargo was
# invoked with --target, so use the qualified path whenever $1 is set.
if [[ -n "${1:-}" ]]; then
    SRC_DIR="target/${TARGET}/release"
else
    SRC_DIR="target/release"
fi

# MSVC emits <name>.exe; Tauri's externalBin then expects binaries/<name>-<triple>.exe.
if [[ "$TARGET" == *windows* ]]; then
    EXE=".exe"
else
    EXE=""
fi

missing=()
for bin in "${SIDECARS[@]}"; do
    [[ -f "$SRC_DIR/${bin}${EXE}" ]] || missing+=("${bin}${EXE}")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    echo "Error: missing release binaries in $SRC_DIR: ${missing[*]}" >&2
    echo "Run 'cargo build --release -p buzz-acp -p buzz-dev-mcp -p git-credential-nostr -p buzz-cli' first." >&2
    exit 1
fi

mkdir -p "$BINARIES_DIR"
for bin in "${SIDECARS[@]}"; do
    cp "$SRC_DIR/${bin}${EXE}" "$BINARIES_DIR/${bin}-${TARGET}${EXE}"
done

# Bundled slim Goose ACP agent. This is the default desktop harness, so a
# missing build is a packaging error rather than an optional fallback.
if [[ -n "${1:-}" ]]; then
    GOOSE_ACP_SRC="$GOOSE_ACP_DIR/target/${TARGET}/release/goose-acp${EXE}"
else
    GOOSE_ACP_SRC="$GOOSE_ACP_DIR/target/release/goose-acp${EXE}"
fi
if [[ ! -f "$GOOSE_ACP_SRC" ]]; then
    echo "Error: missing default Goose harness at $GOOSE_ACP_SRC" >&2
    echo "Run 'cargo build --release --manifest-path goose-acp/Cargo.toml' first." >&2
    exit 1
fi
cp "$GOOSE_ACP_SRC" "$BINARIES_DIR/goose-acp-${TARGET}${EXE}"
# cp onto an existing stub (touched by _ensure-sidecar-stubs) keeps the
# stub's non-executable mode — restore it explicitly.
chmod +x "$BINARIES_DIR/goose-acp-${TARGET}${EXE}"
echo "Bundled goose-acp"
echo "Sidecars bundled for $TARGET"
