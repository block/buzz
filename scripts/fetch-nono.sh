#!/usr/bin/env bash
# Install the pinned nono sandbox binary as a desktop sidecar.
#
# Usage: scripts/fetch-nono.sh [target-triple]   (default: rustc host triple)
#
# Downloads nono-v<version>-<target>.tar.gz from the GitHub release pinned in
# desktop/src-tauri/nono.lock, verifies its SHA-256 against that lock, and
# installs the binary at desktop/src-tauri/binaries/nono-<target>. Tarballs are
# cached under ${XDG_CACHE_HOME:-~/.cache}/buzz/nono and re-verified on every
# run. The sidecar is only rewritten when its content changes: tauri-build
# reruns build.rs whenever an externalBin's mtime changes.
#
# Windows targets are skipped: nono supports Windows only through WSL2, and the
# Windows bundle does not list the nono sidecar.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCK_FILE="$ROOT/desktop/src-tauri/nono.lock"
BINARIES_DIR="$ROOT/desktop/src-tauri/binaries"

TARGET=${1:-$(rustc -vV | sed -n 's|host: ||p')}

if [[ "$TARGET" == *windows* ]]; then
    echo "nono sidecar skipped for $TARGET (no native Windows support)"
    exit 0
fi

VERSION=$(awk '$1 == "version" { print $2 }' "$LOCK_FILE")
EXPECTED=$(awk -v target="$TARGET" '$1 == target { print $2 }' "$LOCK_FILE")
if [[ -z "$VERSION" ]]; then
    echo "Error: no version line in $LOCK_FILE" >&2
    exit 1
fi
if [[ -z "$EXPECTED" ]]; then
    echo "Error: no pinned nono hash for target $TARGET in $LOCK_FILE" >&2
    exit 1
fi

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    else
        shasum -a 256 "$1" | awk '{ print $1 }'
    fi
}

ASSET="nono-v${VERSION}-${TARGET}.tar.gz"
URL="https://github.com/nolabs-ai/nono/releases/download/v${VERSION}/${ASSET}"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/buzz/nono"
TARBALL="$CACHE_DIR/$ASSET"
mkdir -p "$CACHE_DIR"

if [[ ! -f "$TARBALL" || "$(sha256 "$TARBALL")" != "$EXPECTED" ]]; then
    PARTIAL="$TARBALL.partial.$$"
    trap 'rm -f "$PARTIAL"' EXIT
    echo "Downloading $URL"
    curl --fail --silent --show-error --location --retry 3 --output "$PARTIAL" "$URL"
    ACTUAL=$(sha256 "$PARTIAL")
    if [[ "$ACTUAL" != "$EXPECTED" ]]; then
        echo "Error: SHA-256 mismatch for $ASSET" >&2
        echo "  expected $EXPECTED" >&2
        echo "  actual   $ACTUAL" >&2
        exit 1
    fi
    mv "$PARTIAL" "$TARBALL"
    trap - EXIT
fi

WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/buzz-nono.XXXXXX")
trap 'rm -rf "$WORK_DIR"' EXIT
tar -xzf "$TARBALL" -C "$WORK_DIR" nono
EXTRACTED="$WORK_DIR/nono"
if [[ ! -f "$EXTRACTED" ]]; then
    echo "Error: $ASSET does not contain a top-level nono binary" >&2
    exit 1
fi

mkdir -p "$BINARIES_DIR"
DESTINATION="$BINARIES_DIR/nono-${TARGET}"
if [[ -f "$DESTINATION" ]] && cmp -s "$EXTRACTED" "$DESTINATION"; then
    chmod 755 "$DESTINATION"
    echo "nono $VERSION sidecar already current for $TARGET"
    exit 0
fi
# Replace atomically so an interrupted copy never leaves a truncated sidecar.
cp "$EXTRACTED" "$DESTINATION.tmp.$$"
chmod 755 "$DESTINATION.tmp.$$"
mv "$DESTINATION.tmp.$$" "$DESTINATION"
echo "nono $VERSION sidecar installed for $TARGET"
