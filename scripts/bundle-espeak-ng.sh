#!/usr/bin/env bash
set -euo pipefail

VERSION=1.52.0
SHA256=bb4338102ff3b49a81423da8a1a158b420124b055b60fa76cfb4b18677130a23
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST="$ROOT/desktop/src-tauri/resources/espeak-ng"
CACHE="${ESPEAK_NG_CACHE:-/tmp/buzz-espeak-ng-$VERSION}"
TARBALL="$CACHE/espeak-ng-$VERSION.tar.gz"
SRC="$CACHE/espeak-ng-$VERSION"
BUILD="$CACHE/build"

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*|Windows_NT) EXE=espeak-ng.exe ;;
  *) EXE=espeak-ng ;;
esac

mkdir -p "$DEST"
if [[ -x "$DEST/$EXE" && "$(cat "$DEST/.version" 2>/dev/null || true)" == "$VERSION" ]]; then
  exit 0
fi

checksum_ok() {
  printf '%s  %s\n' "$SHA256" "$TARBALL" | sha256sum -c - >/dev/null 2>&1 \
    || printf '%s  %s\n' "$SHA256" "$TARBALL" | shasum -a 256 -c - >/dev/null 2>&1
}

mkdir -p "$CACHE"
if [[ ! -f "$TARBALL" ]] || ! checksum_ok; then
  curl -fsSL -o "$TARBALL" "https://github.com/espeak-ng/espeak-ng/archive/refs/tags/$VERSION.tar.gz"
  checksum_ok || {
    echo "espeak-ng tarball checksum mismatch" >&2
    exit 1
  }
fi

rm -rf "$SRC" "$BUILD"
tar -xzf "$TARBALL" -C "$CACHE"
export CMAKE_POLICY_VERSION_MINIMUM=3.5
cmake -S "$SRC" -B "$BUILD" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DBUILD_TESTING=OFF \
  -DUSE_ASYNC=OFF \
  -DUSE_MBROLA=OFF \
  -DUSE_LIBPCAUDIO=OFF \
  -DUSE_KLATT=OFF \
  -DUSE_SPEECHPLAYER=OFF
cmake --build "$BUILD" --config Release --target espeak-ng-bin --parallel

BIN=$(find "$BUILD" -type f \( -name espeak-ng -o -name espeak-ng.exe \) ! -name '*.a' ! -name '*.lib' | head -1)
if [[ -z "$BIN" ]]; then
  echo "espeak-ng binary was not produced" >&2
  exit 1
fi
cp "$BIN" "$DEST/$EXE"
chmod 755 "$DEST/$EXE"
cp "$SRC/COPYING" "$DEST/COPYING"
printf '%s\n' "$VERSION" > "$DEST/.version"
echo "Bundled $DEST/$EXE"
