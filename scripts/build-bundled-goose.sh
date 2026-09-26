#!/usr/bin/env bash
# Build the internal macOS Goose sidecar from the reviewed source pin.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
source ./bin/activate-hermit
[[ $(uname -s) == Darwin ]] || { echo 'Bundled Goose currently supports macOS only' >&2; exit 1; }
TARGET=${1:-$(rustc -vV | sed -n 's/^host: //p')}
case "$TARGET" in aarch64-apple-darwin|x86_64-apple-darwin) ;; *) echo "Unsupported target: $TARGET" >&2; exit 1 ;; esac
PIN="$ROOT/scripts/goose-build.json"
REPOSITORY=$(node -p 'require(process.argv[1]).repository' "$PIN")
REVISION=$(node -p 'require(process.argv[1]).revision' "$PIN")
PROFILE=$(node -p 'require(process.argv[1]).profile' "$PIN")
FEATURES=$(node -p 'require(process.argv[1]).features' "$PIN")
[[ "$REVISION" =~ ^[0-9a-f]{40}$ && "$PROFILE" == lean ]] || { echo 'Invalid Goose source pin' >&2; exit 1; }
CACHE="$ROOT/.cache/bundled-goose/$REVISION"
SOURCE="$CACHE/source"
mkdir -p "$CACHE"
# Never share a mutable checkout or target directory with the developer's Goose.
if [[ ! -d "$SOURCE/.git" ]]; then
  git init -q "$SOURCE"
fi
if ! git -C "$SOURCE" cat-file -e "$REVISION^{commit}" 2>/dev/null; then
  git -C "$SOURCE" -c http.lowSpeedLimit=1024 -c http.lowSpeedTime=60 fetch --depth=1 "$REPOSITORY" "$REVISION"
fi
[[ -z $(git -C "$SOURCE" status --porcelain) ]] || { echo "Dirty Goose build checkout: $SOURCE" >&2; exit 1; }
git -C "$SOURCE" checkout --detach "$REVISION"
[[ $(git -C "$SOURCE" rev-parse HEAD) == "$REVISION" ]]
(
  cd "$SOURCE"
  source ./bin/activate-hermit
  TOOLCHAIN=$(rustc -vV | shasum -a 256 | cut -d ' ' -f 1)
  export CARGO_TARGET_DIR="$CACHE/target/$TOOLCHAIN/$FEATURES"
  # Avoid a runtime dependency on a build machine's Homebrew libssl/libcrypto.
  export OPENSSL_STATIC=1
  cargo build --locked -p goose --bin goose-acp --profile "$PROFILE" \
    --no-default-features --features "$FEATURES" --target "$TARGET"
  BINARY="$CARGO_TARGET_DIR/$TARGET/$PROFILE/goose-acp"
  "$ROOT/scripts/verify-bundled-goose.sh" "$BINARY"
  DEST="$ROOT/desktop/src-tauri/binaries/goose-acp-$TARGET"
  mkdir -p "$(dirname "$DEST")"
  # Atomic replacement keeps an already-running signed executable valid.
  STAGED=$(mktemp "$DEST.XXXXXX")
  trap 'rm -f "$STAGED"' EXIT
  cp "$BINARY" "$STAGED"
  chmod 755 "$STAGED"
  mv -f "$STAGED" "$DEST"
  node - "$PIN" "$DEST" "$TARGET" "$(rustc --version)" <<'JS'
const fs = require('node:fs');
const crypto = require('node:crypto');
const [pin, binary, target, rustc] = process.argv.slice(2);
const manifest = {
  ...JSON.parse(fs.readFileSync(pin, 'utf8')), target, rustc,
  unsigned_sha256: crypto.createHash('sha256').update(fs.readFileSync(binary)).digest('hex'),
};
fs.writeFileSync(`${binary}.json`, `${JSON.stringify(manifest, null, 2)}\n`);
JS
  echo "Staged $DEST"
)
