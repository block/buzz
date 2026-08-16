#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
checker="${repo_root}/scripts/check-adaptive-memory.sh"
test_tmp="$(mktemp -d)"
mock_bin="${test_tmp}/bin"
command_log="${test_tmp}/commands.log"

cleanup() {
  rm -rf "${test_tmp}"
}
trap cleanup EXIT

fail() {
  printf 'check-adaptive-memory test failed: %s\n' "$*" >&2
  exit 1
}

mkdir -p "${mock_bin}"
mkdir -p "${test_tmp}/memory/MemoryMCPServer"
for command_name in cargo python3; do
  printf '%s\n' \
    '#!/bin/bash' \
    'set -euo pipefail' \
    'printf "%s %s\n" "$(basename "$0")" "$*" >>"${ADAPTIVE_MEMORY_MOCK_LOG}"' \
    'if [[ -n "${ADAPTIVE_MEMORY_FAIL_MATCH:-}" && "$*" == *"${ADAPTIVE_MEMORY_FAIL_MATCH}"* ]]; then' \
    '  exit 17' \
    'fi' \
    >"${mock_bin}/${command_name}"
  chmod +x "${mock_bin}/${command_name}"
done

PATH="${mock_bin}:${PATH}" \
ADAPTIVE_MEMORY_MOCK_LOG="${command_log}" \
CARGO="${mock_bin}/cargo" \
PYTHON="${mock_bin}/python3" \
MEMORY_MCP_REPO="${test_tmp}/memory" \
  /bin/bash "${checker}" >"${test_tmp}/success.out"

grep -Fq 'cargo test -p buzz-core agent_experience' "${command_log}" ||
  fail "experience contract tests were not run"
grep -Fq 'cargo test -p buzz-acp experience_' "${command_log}" ||
  fail "durable capture and projection tests were not run"
grep -Fq 'cargo test -p buzz-acp engram_recall' "${command_log}" ||
  fail "selective recall tests were not run"
grep -Fq 'cargo test -p buzz-cli mem' "${command_log}" ||
  fail "history and rebuild CLI tests were not run"
grep -Fq 'python3 -m pytest -q' "${command_log}" ||
  fail "Memory MCP projection and active-view tests were not run"
grep -Fq 'all adaptive-memory checks passed' "${test_tmp}/success.out" ||
  fail "success evidence was not printed"

if PATH="${mock_bin}:${PATH}" \
  ADAPTIVE_MEMORY_MOCK_LOG="${command_log}" \
  ADAPTIVE_MEMORY_FAIL_MATCH="engram_recall" \
  CARGO="${mock_bin}/cargo" \
  PYTHON="${mock_bin}/python3" \
  MEMORY_MCP_REPO="${test_tmp}/memory" \
    /bin/bash "${checker}" >"${test_tmp}/failure.out" 2>&1; then
  fail "a failed recall check did not fail the acceptance runner"
fi
if grep -Fq 'all adaptive-memory checks passed' "${test_tmp}/failure.out"; then
  fail "failed acceptance run printed a success claim"
fi

printf 'check-adaptive-memory orchestration contract passed\n'
