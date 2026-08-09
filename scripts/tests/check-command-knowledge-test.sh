#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
checker="${repo_root}/scripts/check-command-knowledge.sh"
test_tmp="$(mktemp -d)"
mock_bin="${test_tmp}/bin"
command_log="${test_tmp}/commands.log"

cleanup() {
  rm -rf "${test_tmp}"
}
trap cleanup EXIT

fail() {
  printf 'check-command-knowledge test failed: %s\n' "$*" >&2
  exit 1
}

mkdir -p "${mock_bin}"
for command_name in bash cargo pnpm xcodebuild; do
  printf '%s\n' \
    '#!/bin/bash' \
    'set -euo pipefail' \
    'printf "%s %s\n" "$(basename "$0")" "$*" >>"${COMMAND_KNOWLEDGE_MOCK_LOG}"' \
    'if [[ -n "${COMMAND_KNOWLEDGE_FAIL_MATCH:-}" && "$*" == *"${COMMAND_KNOWLEDGE_FAIL_MATCH}"* ]]; then' \
    '  exit 17' \
    'fi' \
    >"${mock_bin}/${command_name}"
  chmod +x "${mock_bin}/${command_name}"
done

PATH="${mock_bin}:${PATH}" \
COMMAND_KNOWLEDGE_MOCK_LOG="${command_log}" \
CARGO="${mock_bin}/cargo" \
PNPM="${mock_bin}/pnpm" \
XCODEBUILD="${mock_bin}/xcodebuild" \
  /bin/bash "${checker}" >"${test_tmp}/success.out"

grep -Fq 'bash scripts/tests/command-memory-service-test.sh' "${command_log}" ||
  fail "Memory topology fixture was not run"
grep -Fq 'bash scripts/tests/check-lmstudio-native-test.sh' "${command_log}" ||
  fail "LM Studio fixture was not run"
grep -Fq 'cargo test -p buzz-core agent_memory_canonical --lib' "${command_log}" ||
  fail "canonical JSON check was not run"
grep -Fq 'malicious_native_mcp_evidence_is_blocked_before_any_continuation_request' \
  "${command_log}" ||
  fail "continuation-state evidence check was not run"
grep -Fq 'command_services::memory::replication::tests -- --test-threads=1' \
  "${command_log}" ||
  fail "Memory replication checks were not run"
grep -Fq 'command_services::memory::sync_state::tests -- --test-threads=1' \
  "${command_log}" ||
  fail "Memory sync-state checks were not run"
grep -Fq 'command_services::memory::memory_tests::tests -- --test-threads=1' \
  "${command_log}" ||
  fail "Memory credential-admission checks were not run"
grep -Fq 'command_services::policy::tests -- --test-threads=1' "${command_log}" ||
  fail "service-admission checks were not run"
grep -Fq 'command_services::rag::tests -- --test-threads=1' "${command_log}" ||
  fail "RAG readiness checks were not run"
grep -Fq 'pnpm --dir desktop test' "${command_log}" ||
  fail "Command Console tests were not run"
grep -Fq 'xcodebuild test -project desktop/apple-inputs/BuzzAppleInputs.xcodeproj' \
  "${command_log}" ||
  fail "Apple helper tests were not run"
grep -Fq 'all hermetic Phase 3 knowledge checks passed' "${test_tmp}/success.out" ||
  fail "success evidence was not printed"

if PATH="${mock_bin}:${PATH}" \
  COMMAND_KNOWLEDGE_MOCK_LOG="${command_log}" \
  COMMAND_KNOWLEDGE_FAIL_MATCH="command_services::policy::tests" \
  CARGO="${mock_bin}/cargo" \
  PNPM="${mock_bin}/pnpm" \
  XCODEBUILD="${mock_bin}/xcodebuild" \
    /bin/bash "${checker}" >"${test_tmp}/failure.out" 2>&1; then
  fail "a failed admission check did not fail the acceptance runner"
fi
if grep -Fq 'all hermetic Phase 3 knowledge checks passed' "${test_tmp}/failure.out"; then
  fail "failed acceptance run printed a success claim"
fi

printf 'check-command-knowledge orchestration contract passed\n'
