#!/usr/bin/env bash
# Stage built debug sidecars into a destination directory.
# Usage: stage-debug-sidecars.sh <target-triple> <cargo-target-dir> <dest-dir>
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "Usage: $0 <target-triple> <cargo-target-dir> <dest-dir>" >&2
  exit 2
fi

TARGET="$1"
TARGET_DIR="${2//\\//}"
DEST_DIR="$3"
SIDECARS=(buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz)
EXE_SUFFIX=""
if [[ "$TARGET" == *windows* ]]; then
  EXE_SUFFIX=".exe"
else
  SIDECARS+=(buzz-backend-kubernetes)
fi

mkdir -p "$DEST_DIR"
for bin in "${SIDECARS[@]}"; do
  source_path="${TARGET_DIR}/debug/${bin}${EXE_SUFFIX}"
  if [[ ! -f "$source_path" ]]; then
    echo "Error: expected debug sidecar is missing: $source_path" >&2
    exit 1
  fi
  if [[ ! -s "$source_path" ]]; then
    echo "Error: expected debug sidecar is empty: $source_path" >&2
    exit 1
  fi
done
for bin in "${SIDECARS[@]}"; do
  source_path="${TARGET_DIR}/debug/${bin}${EXE_SUFFIX}"
  destination_path="${DEST_DIR}/${bin}-${TARGET}${EXE_SUFFIX}"
  cp "$source_path" "$destination_path"
  if [[ "$TARGET" != *windows* ]]; then
    chmod +x "$destination_path"
  fi
done
