#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cargo_bin="${CARGO:-cargo}"
pnpm_bin="${PNPM:-pnpm}"
xcodebuild_bin="${XCODEBUILD:-xcodebuild}"

run_check() {
  local label="$1"
  shift
  printf '[command-knowledge] %s\n' "${label}"
  "$@"
}

cd "${repo_root}"

run_check \
  "Mac-local Memory topology and encrypted backup contract" \
  bash scripts/tests/command-memory-service-test.sh
run_check \
  "LM Studio loopback, proxy, auth, response-bound, and structured-tool fixtures" \
  bash scripts/tests/check-lmstudio-native-test.sh
run_check \
  "AgentMemory canonical JSON compatibility" \
  "${cargo_bin}" test -p buzz-core agent_memory_canonical --lib
run_check \
  "Native command evidence policy" \
  "${cargo_bin}" test -p buzz-agent command_evidence --lib
run_check \
  "Rejected LM Studio evidence cannot enter continuation state" \
  "${cargo_bin}" test -p buzz-agent --test lmstudio_native \
    malicious_native_mcp_evidence_is_blocked_before_any_continuation_request
run_check \
  "Cancellable and conflict-safe Memory replication" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_services::memory::replication::tests -- --test-threads=1
run_check \
  "Truthful persisted Memory sync status" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_services::memory::sync_state::tests -- --test-threads=1
run_check \
  "Authenticated Memory configuration and service admission" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_services::memory::memory_tests::tests -- --test-threads=1
run_check \
  "Authenticated Memory and RAG admission policy" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_services::policy::tests -- --test-threads=1
run_check \
  "Signed RAG snapshot readiness contract" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_services::rag::tests -- --test-threads=1
run_check \
  "Command Console knowledge-status parsing and presentation" \
  "${pnpm_bin}" --dir desktop test
run_check \
  "Read-only Apple input helper fixtures" \
  env DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
    "${xcodebuild_bin}" test \
      -project desktop/apple-inputs/BuzzAppleInputs.xcodeproj \
      -scheme BuzzAppleInputs \
      -destination platform=macOS \
      CODE_SIGNING_ALLOWED=NO

printf '[command-knowledge] all hermetic Phase 3 knowledge checks passed\n'
