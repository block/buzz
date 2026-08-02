#!/usr/bin/env bash
set -euo pipefail

SIDECARS=(buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz buzz-guardian-numbat)
HOST=$(rustc -vV | sed -n 's|host: ||p')
TARGET=${1:-$HOST}
if [[ "$TARGET" != *windows* ]]; then
    SIDECARS+=(buzz-backend-kubernetes)
    BUILD_HINT="cargo build --release -p buzz-acp -p buzz-agent -p buzz-backend-kubernetes -p buzz-dev-mcp -p git-credential-nostr -p buzz-cli"
else
    BUILD_HINT="cargo build --release -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p git-credential-nostr -p buzz-cli"
fi
BINARIES_DIR="desktop/src-tauri/binaries"

# When --target is passed explicitly to cargo (even if it matches the host),
# binaries land in target/<triple>/release/. Without --target, they land in
# target/release/. The script receives the target as $1 only when cargo was
# invoked with --target, so use the qualified path whenever $1 is set.
if [[ -n "${1:-}" ]]; then
    SRC_DIR="target/${TARGET}/release"
    DESKTOP_SRC_DIR="desktop/src-tauri/target/${TARGET}/release"
else
    SRC_DIR="target/release"
    DESKTOP_SRC_DIR="desktop/src-tauri/target/release"
fi

# MSVC emits <name>.exe; Tauri's externalBin then expects binaries/<name>-<triple>.exe.
if [[ "$TARGET" == *windows* ]]; then
    EXE=".exe"
else
    EXE=""
fi

missing=()
for bin in "${SIDECARS[@]}"; do
    if [[ "$bin" == "buzz-guardian-numbat" ]]; then
        source_dir="$DESKTOP_SRC_DIR"
    else
        source_dir="$SRC_DIR"
    fi
    [[ -f "$source_dir/${bin}${EXE}" ]] || missing+=("${bin}${EXE}")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    echo "Error: missing release binaries: ${missing[*]}" >&2
    echo "Run '$BUILD_HINT' first." >&2
    echo "Build Guardian with 'cargo build --release --manifest-path desktop/src-tauri/Cargo.toml --bin buzz-guardian-numbat'." >&2
    exit 1
fi

mkdir -p "$BINARIES_DIR"
for bin in "${SIDECARS[@]}"; do
    if [[ "$bin" == "buzz-guardian-numbat" ]]; then
        source_dir="$DESKTOP_SRC_DIR"
    else
        source_dir="$SRC_DIR"
    fi
    destination="$BINARIES_DIR/${bin}-${TARGET}${EXE}"
    cp "$source_dir/${bin}${EXE}" "$destination"

    # cp preserves the mode of an existing destination on macOS. Generated
    # sidecar placeholders may not be executable, so make the bundled Unix
    # binaries executable explicitly.
    if [[ -z "$EXE" ]]; then
        chmod 755 "$destination"
    fi
done
echo "Sidecars bundled for $TARGET"
