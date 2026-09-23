#!/usr/bin/env bash
set -euo pipefail

# Keep this pinned: upstream's lean ACP binary is the desktop Goose runtime.
GOOSE_REV="e678c3b64a1dfd3c262a6a2019f158d33d5dcab0"
TARGET="${1:-}"
ROOT=$(pwd)
WORK_DIR="${ROOT}/target/goose-src"

if [[ ! -d "$WORK_DIR/.git" ]]; then
    mkdir -p "$WORK_DIR"
    git -C "$WORK_DIR" init --quiet
    git -C "$WORK_DIR" remote add origin https://github.com/aaif-goose/goose.git
fi

git -C "$WORK_DIR" fetch --quiet --depth 1 origin "$GOOSE_REV"
git -C "$WORK_DIR" checkout --quiet --detach FETCH_HEAD

CARGO=(cargo build -p goose --bin goose-acp --profile lean --no-default-features --features rustls-tls)
if [[ -n "$TARGET" ]]; then
    CARGO+=(--target "$TARGET")
fi
(
    cd "$WORK_DIR"
    "${CARGO[@]}"
)

SRC_DIR="$WORK_DIR/target"
if [[ -n "$TARGET" ]]; then
    SRC_DIR+="/$TARGET"
fi
SRC_DIR+="/lean"
EXE=""
if [[ "$TARGET" == *windows* ]]; then
    EXE=".exe"
fi
DEST_DIR="${ROOT}/target/${TARGET:+$TARGET/}release"
mkdir -p "$DEST_DIR"
install -m 755 "$SRC_DIR/goose-acp$EXE" "$DEST_DIR/goose-acp$EXE"
