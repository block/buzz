#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   bundle-sidecars.sh [TARGET]                         stage built sidecars into desktop/src-tauri/binaries
#   bundle-sidecars.sh --print-cargo-packages [TARGET]  print the `cargo build -p ...` args for TARGET
PRINT_CARGO_PACKAGES=false
if [[ "${1:-}" == "--print-cargo-packages" ]]; then
    PRINT_CARGO_PACKAGES=true
    shift
fi

SIDECARS=(buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz)
if [[ -n "${1:-}" ]]; then
    TARGET="$1"
else
    HOST=$(rustc -vV 2>/dev/null | sed -n 's|host: ||p')
    TARGET="${HOST:-}"
    if [[ -z "$TARGET" ]]; then
        echo "Error: target not specified and rustc is not available to determine host triple" >&2
        exit 1
    fi
fi
if [[ "$TARGET" != *windows* ]]; then
    SIDECARS+=(buzz-backend-kubernetes)
fi

# Cargo package for each sidecar binary (the `buzz` binary is built by buzz-cli).
CARGO_PACKAGE_ARGS=()
for bin in "${SIDECARS[@]}"; do
    if [[ "$bin" == "buzz" ]]; then
        CARGO_PACKAGE_ARGS+=(-p buzz-cli)
    else
        CARGO_PACKAGE_ARGS+=(-p "$bin")
    fi
done
if [[ "$PRINT_CARGO_PACKAGES" == true ]]; then
    echo "${CARGO_PACKAGE_ARGS[*]}"
    exit 0
fi
BUILD_HINT="cargo build --release ${CARGO_PACKAGE_ARGS[*]}"
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
    [[ -s "$SRC_DIR/${bin}${EXE}" ]] || missing+=("${bin}${EXE}")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    echo "Error: missing or empty release binaries in $SRC_DIR: ${missing[*]}" >&2
    echo "Run '$BUILD_HINT' first." >&2
    exit 1
fi

mkdir -p "$BINARIES_DIR"
for bin in "${SIDECARS[@]}"; do
    destination="$BINARIES_DIR/${bin}-${TARGET}${EXE}"
    cp "$SRC_DIR/${bin}${EXE}" "$destination"

    # cp preserves the mode of an existing destination on macOS. Generated
    # sidecar placeholders may not be executable, so make the bundled Unix
    # binaries executable explicitly.
    if [[ -z "$EXE" ]]; then
        chmod 755 "$destination"
    fi

    if [[ ! -s "$destination" ]]; then
        echo "Error: staged sidecar $destination is missing or empty" >&2
        exit 1
    fi
    if [[ -z "$EXE" && ! -x "$destination" ]]; then
        echo "Error: staged sidecar $destination is not executable" >&2
        exit 1
    fi
done
echo "Sidecars bundled for $TARGET"
