#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
. ./bin/activate-hermit

BINARY="tools/tts-tester/target/release/buzz-tts-tester"
NEEDS_BUILD=0

if [[ ! -x "$BINARY" ]]; then
  NEEDS_BUILD=1
else
  for SOURCE in \
    tools/tts-tester/Cargo.toml \
    tools/tts-tester/Cargo.lock \
    tools/tts-tester/src/*.rs \
    desktop/src-tauri/src/huddle/pocket.rs \
    desktop/src-tauri/src/huddle/preprocessing.rs
  do
    if [[ "$SOURCE" -nt "$BINARY" ]]; then
      NEEDS_BUILD=1
      break
    fi
  done
fi

if [[ "$NEEDS_BUILD" -eq 1 ]]; then
  cargo build \
    --offline \
    --locked \
    --release \
    --manifest-path tools/tts-tester/Cargo.toml
fi

exec "$BINARY" "$@"
