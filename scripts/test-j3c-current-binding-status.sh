#!/usr/bin/env bash

set -euo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
artifact_dir="$(mktemp -d "${TMPDIR:-/tmp}/buzz-j3c-binding-status.XXXXXX")"
trace_path="$artifact_dir/current-binding-status-projection.json"

cd "$repo_root"

TAURI_CONFIG='{"bundle":{"externalBin":[]}}' \
BUZZ_J3C_PROJECTION_TRACE_OUT="$trace_path" \
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}" \
  bin/cargo test \
    --manifest-path desktop/src-tauri/Cargo.toml \
    --test current_binding_status_native_flow \
    --locked \
    loopback_relay_drives_production_projection_and_trace \
    -- \
    --exact \
    --nocapture

test -s "$trace_path"

cd "$repo_root/desktop"
NODE_OPTIONS="${NODE_OPTIONS:---max-old-space-size=2048}" \
  ../bin/pnpm build:e2e
BUZZ_J3C_PROJECTION_TRACE="$trace_path" \
  ../bin/pnpm exec playwright test --config=playwright.j3c.config.ts

printf 'J3C projection trace: %s\n' "$trace_path"
