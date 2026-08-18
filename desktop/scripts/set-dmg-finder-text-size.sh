#!/usr/bin/env bash
# Patch Finder icon-label text size in a Tauri-generated DMG.
#
# Tauri's DMG config supports the background/layout fields we use, but it does
# not expose create-dmg's text-size option. Run this before DMG signing or
# notarization; mutating a signed DMG would invalidate the signature.

set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <path-to-dmg> [text-size]" >&2
  exit 2
fi

DMG_PATH="$1"
TEXT_SIZE="${2:-14}"

[[ -f "$DMG_PATH" ]] || { echo "Missing DMG: $DMG_PATH" >&2; exit 1; }

WORK_DIR="$(mktemp -d -t buzz-dmg-text-size)"
MOUNT_POINT="$WORK_DIR/mount"
RW_DMG="$WORK_DIR/work.dmg"
OUT_DMG="$WORK_DIR/output.dmg"
MOUNTED="false"

cleanup() {
  if [[ "$MOUNTED" == "true" ]]; then
    hdiutil detach "$MOUNT_POINT" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

mkdir -p "$MOUNT_POINT"
hdiutil convert "$DMG_PATH" -format UDRW -o "$RW_DMG" -ov >/dev/null
hdiutil attach "$RW_DMG" -nobrowse -readwrite -noverify -mountpoint "$MOUNT_POINT" >/dev/null
MOUNTED="true"

node - "$MOUNT_POINT/.DS_Store" "$TEXT_SIZE" <<'NODE'
const fs = require("node:fs");

const dsStorePath = process.argv[2];
const textSize = Number(process.argv[3]);

if (!Number.isFinite(textSize)) {
  throw new Error(`Invalid DMG Finder text size: ${process.argv[3]}`);
}

const data = fs.readFileSync(dsStorePath);
if (!data.includes(Buffer.from("textSize"))) {
  throw new Error(`${dsStorePath} does not contain Finder textSize metadata`);
}

const current = Buffer.alloc(8);
current.writeDoubleBE(16.0, 0);
const replacement = Buffer.alloc(8);
replacement.writeDoubleBE(textSize, 0);

const positions = [];
let offset = 0;
while ((offset = data.indexOf(current, offset)) !== -1) {
  positions.push(offset);
  offset += 1;
}

if (positions.length !== 1) {
  throw new Error(
    `Expected one Finder textSize value in ${dsStorePath}, found ${positions.length}`,
  );
}

replacement.copy(data, positions[0]);
fs.writeFileSync(dsStorePath, data);
console.log(`Set DMG Finder textSize to ${textSize}`);
NODE

sync
hdiutil detach "$MOUNT_POINT" >/dev/null
MOUNTED="false"

hdiutil convert "$RW_DMG" -format UDZO -imagekey zlib-level=9 -o "$OUT_DMG" -ov >/dev/null
mv "$OUT_DMG" "$DMG_PATH"
