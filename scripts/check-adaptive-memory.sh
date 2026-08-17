#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
memory_repo="${MEMORY_MCP_REPO:-${repo_root}/../Memory MCP}"
cargo_bin="${CARGO:-cargo}"
python_bin="${PYTHON:-python3}"

run_check() {
  local label="$1"
  shift
  printf '[adaptive-memory] %s\n' "${label}"
  "$@"
}

[[ -d "${memory_repo}/MemoryMCPServer" ]] || {
  printf 'Memory MCP repository not found: %s\n' "${memory_repo}" >&2
  exit 2
}

cd "${repo_root}"
run_check "encrypted experience contract" \
  "${cargo_bin}" test -p buzz-core agent_experience
run_check "durable capture and idempotent projection" \
  "${cargo_bin}" test -p buzz-acp experience_
run_check "bounded active recall" \
  "${cargo_bin}" test -p buzz-acp engram_recall
run_check "history and rebuild commands" \
  "${cargo_bin}" test -p buzz-cli mem

printf '[adaptive-memory] Memory MCP active-view and projection contracts\n'
(
  cd "${memory_repo}/MemoryMCPServer"
  "${python_bin}" -m pytest -q tests/test_projected_events.py tests/test_active_memory.py
)

printf '[adaptive-memory] all adaptive-memory checks passed\n'
