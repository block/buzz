#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
orchestrator="$repo_root/.github/workflows/ci.yml"
rust_workflow="$repo_root/.github/workflows/_ci-rust.yml"
relay_workflow="$repo_root/.github/workflows/_ci-relay.yml"
clients_workflow="$repo_root/.github/workflows/_ci-clients.yml"

fail() {
  echo "CI required-context isolation contract failed: $*" >&2
  exit 1
}

extract_job() {
  local job=$1
  local workflow=$2
  awk -v job="$job" '
    $0 == "  " job ":" { found = 1 }
    found && $0 ~ /^  [a-zA-Z0-9_-]+:$/ && $0 != "  " job ":" { exit }
    found { print }
  ' "$workflow"
}

rust_call=$(extract_job rust "$orchestrator")
rust_cross_compile_call=$(extract_job rust-cross-compile-domain "$orchestrator")
relay_call=$(extract_job relay-domain "$orchestrator")
relay_artifacts_call=$(extract_job relay-artifacts-domain "$orchestrator")
postgres_call=$(extract_job postgres-domain "$orchestrator")
clients_call=$(extract_job clients "$orchestrator")
mobile_swift_call=$(extract_job mobile-swift-domain "$orchestrator")
relay_e2e_job=$(extract_job relay-e2e "$relay_workflow")

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
[[ "$relay_e2e_job" == *'cargo test -p buzz-test-client --test e2e_relay nip29_departure_wire -- --ignored --nocapture'* ]] ||
  fail "Relay E2E must select the NIP-29 departure wire tests"
grep -Fq "inputs.lane == 'mobile-swift'" "$clients_workflow" ||
  fail "Mobile Swift must be selected by its isolated lane"

echo "CI required-context isolation contract passed"
