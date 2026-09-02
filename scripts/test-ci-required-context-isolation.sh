#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
orchestrator="$repo_root/.github/workflows/ci.yml"
desktop_workflow="$repo_root/.github/workflows/_ci-desktop.yml"
macos_workflow="$repo_root/.github/workflows/_ci-desktop-macos.yml"
rust_workflow="$repo_root/.github/workflows/_ci-rust.yml"
relay_workflow="$repo_root/.github/workflows/_ci-relay.yml"
clients_workflow="$repo_root/.github/workflows/_ci-clients.yml"

fail() {
  echo "CI required-context isolation contract failed: $*" >&2
  exit 1
}

[[ -f "$macos_workflow" ]] || fail "missing _ci-desktop-macos.yml"

extract_job() {
  local job=$1
  local workflow=$2
  awk -v job="$job" '
    $0 == "  " job ":" { found = 1 }
    found && $0 ~ /^  [a-zA-Z0-9_-]+:$/ && $0 != "  " job ":" { exit }
    found { print }
  ' "$workflow"
}

desktop_call=$(extract_job desktop-domain "$orchestrator")
macos_call=$(extract_job desktop-macos-domain "$orchestrator")
rust_call=$(extract_job rust "$orchestrator")
rust_cross_compile_call=$(extract_job rust-cross-compile-domain "$orchestrator")
relay_call=$(extract_job relay-domain "$orchestrator")
relay_artifacts_call=$(extract_job relay-artifacts-domain "$orchestrator")
postgres_call=$(extract_job postgres-domain "$orchestrator")
clients_call=$(extract_job clients "$orchestrator")
mobile_swift_call=$(extract_job mobile-swift-domain "$orchestrator")
desktop_gate=$(extract_job desktop "$orchestrator")
macos_gate=$(extract_job desktop-build-macos "$orchestrator")

[[ "$desktop_call" == *'uses: ./.github/workflows/_ci-desktop.yml'* ]] ||
  fail "Desktop Domain must call _ci-desktop.yml"
[[ "$macos_call" == *'uses: ./.github/workflows/_ci-desktop-macos.yml'* ]] ||
  fail "Desktop macOS domain must call _ci-desktop-macos.yml"
[[ "$desktop_gate" == *'needs: [changes, desktop-domain]'* ]] ||
  fail "Desktop required check must depend only on Desktop Domain"
[[ "$desktop_gate" == *'needs.desktop-domain.outputs.desktop_result'* ]] ||
  fail "Desktop required check must read the Desktop Domain result"
[[ "$macos_gate" == *'needs: [changes, desktop-macos-domain]'* ]] ||
  fail "Desktop Build (macOS) required check must depend only on the macOS domain"
[[ "$macos_gate" == *'needs.desktop-macos-domain.outputs.desktop_macos_result'* ]] ||
  fail "Desktop Build (macOS) required check must read the macOS domain result"

[[ "$rust_call" == *'lane: required'* && "$rust_cross_compile_call" == *'lane: cross-compile'* ]] ||
  fail "required Rust checks and server cross-compiles must use isolated calls"
[[ "$relay_artifacts_call" == *'lane: artifacts'* && "$relay_call" == *'lane: required'* && "$postgres_call" == *'lane: postgres'* ]] ||
  fail "relay artifacts, required checks, and PostgreSQL tests must use isolated calls"
[[ "$relay_call" == *'needs: [changes, relay-artifacts-domain]'* ]] ||
  fail "required relay checks must wait for the artifact producer"
[[ "$postgres_call" == *'needs: [changes, relay-artifacts-domain]'* ]] ||
  fail "PostgreSQL tests must wait for the artifact producer"
[[ "$clients_call" == *'lane: required'* && "$mobile_swift_call" == *'lane: mobile-swift'* ]] ||
  fail "required client checks and Mobile Swift must use isolated calls"

grep -Fq "inputs.lane == 'cross-compile'" "$rust_workflow" ||
  fail "Rust cross-compiles must be selected by their isolated lane"
grep -Fq "inputs.lane == 'postgres'" "$relay_workflow" ||
  fail "PostgreSQL tests must be selected by their isolated lane"
grep -Fq "inputs.lane == 'artifacts'" "$relay_workflow" ||
  fail "relay artifacts must be selected by their isolated lane"
grep -Fq "inputs.lane == 'mobile-swift'" "$clients_workflow" ||
  fail "Mobile Swift must be selected by its isolated lane"

grep -Fq 'on:' "$macos_workflow" || fail "macOS workflow is missing its trigger"
grep -Fq '  workflow_call:' "$macos_workflow" || fail "macOS workflow must use workflow_call"
grep -Fq '  desktop-build-macos:' "$macos_workflow" || fail "macOS build job is missing"
grep -Fq 'desktop_macos_result:' "$macos_workflow" || fail "macOS result output is missing"

if grep -Eq '^  desktop-build-macos:|desktop_macos_result:' "$desktop_workflow"; then
  fail "Desktop Domain must not own the isolated macOS required check"
fi

echo "CI required-context isolation contract passed"
