#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

failed=0

reject() {
  local label="$1"
  local pattern="$2"
  shift 2
  local matches
  matches="$(rg -n "$pattern" "$@" || true)"
  if [[ -n "$matches" ]]; then
    echo "FAIL: $label"
    echo "$matches"
    failed=1
  fi
}

reject \
  "Buzz ACP core contains business-specific policy" \
  'Business(Response|Agent|Audit)|business_(response|agent|audit)|business-read' \
  crates/buzz-acp/src/acp.rs \
  crates/buzz-acp/src/lib.rs \
  crates/buzz-acp/src/pool.rs \
  crates/buzz-acp/src/turn_observer.rs

reject \
  "Buzz desktop core imports business implementation directly" \
  'BusinessDock|WorkbenchAuth|features/business-dock|features/workbench-auth' \
  desktop/src/app/AppShell.tsx \
  desktop/src/app/AppTopChrome.tsx \
  desktop/src/main.tsx

reject \
  "Buzz app directory contains business-owned implementation" \
  'BusinessDock|WorkbenchAuth|business[_-](dock|auth|iam)' \
  desktop/src/app

reject \
  "Business IAM policy core depends on Buzz, transport, or persistence" \
  'buzz-(core|acp|sdk)|nostr|axum|reqwest|sqlx' \
  crates/business-iam/Cargo.toml \
  crates/business-iam/src

if [[ "$failed" -ne 0 ]]; then
  echo "Business extension boundary check failed."
  exit 1
fi

echo "PASS: Business extension boundaries are intact."
