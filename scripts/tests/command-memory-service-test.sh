#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
compose_file="${repo_root}/docker-compose.yml"
checker="${repo_root}/scripts/check-local-services.sh"
backup_script="${repo_root}/scripts/backup-local-workspace.sh"
restore_script="${repo_root}/scripts/restore-local-workspace.sh"

fail() {
  printf 'not ok - %s\n' "$*" >&2
  exit 1
}

memory_block="$(
  sed -n '/^  memory:/,/^  [a-z][a-z-]*:/p' "${compose_file}" |
    sed '$d'
)"
[[ -n "${memory_block}" ]] || fail "Compose defines the Mac-local Memory service"
grep -Fq 'profiles: ["command-memory"]' <<<"${memory_block}" ||
  fail "Memory topology is opt-in and does not claim deployment"
grep -Fq '127.0.0.1:${BUZZ_MEMORY_PORT:-18006}:8006' <<<"${memory_block}" ||
  fail "Memory publishes only on literal host loopback"
if grep -Eq '(^|[[:space:]-])("?)(0\.0\.0\.0|::):.*8006' <<<"${memory_block}"; then
  fail "Memory must not publish on a wildcard host address"
fi
grep -Fq 'MEMORY_VAULT_ROOT: /data/current' <<<"${memory_block}" ||
  fail "Memory uses a persistent canonical vault"
grep -Fq 'MEMORY_INDEX_ROOT: /data/index' <<<"${memory_block}" ||
  fail "Memory keeps the rebuildable index separate"
grep -Fq 'MEMORY_NODE_ID: ${BUZZ_MEMORY_NODE_ID:-node:macbook-command}' \
  <<<"${memory_block}" ||
  fail "Memory has a stable local node identity"
grep -Fq 'MEMORY_MCP_REQUIRE_AUTH: "true"' <<<"${memory_block}" ||
  fail "Memory MCP authentication is mandatory in the command profile"
grep -Fq 'MEMORY_ATTESTATION_SECRET="$$(cat /run/secrets/memory-attestation-secret)"' \
  <<<"${memory_block}" ||
  fail "Memory loads a separate server-attestation secret"
grep -Fq 'memory-attestation-secret' <<<"${memory_block}" ||
  fail "Memory mounts the server-attestation secret"
grep -Fq 'next(k for k,v in tokens.items() if \"read\" in v)' <<<"${memory_block}" ||
  fail "Memory readiness derives a read token from token-to-capability JSON"
grep -Fq '/replication/readiness' <<<"${memory_block}" ||
  fail "Memory has an authenticated replication readiness probe"
grep -Fq 'memory-vault:/data' <<<"${memory_block}" ||
  fail "Memory mounts the canonical vault volume"
grep -Fq 'memory-index:/data/index' <<<"${memory_block}" ||
  fail "Memory mounts the rebuildable index volume"
grep -Fq 'memory-attestation-secret:' "${compose_file}" ||
  fail "Compose declares the separate Memory attestation secret"
grep -Fq 'BUZZ_MEMORY_ATTESTATION_SECRET_FILE' "${compose_file}" ||
  fail "Memory attestation secret path is operator-configurable"

grep -Eq 'default_optional_services=.*memory' "${checker}" ||
  fail "local service readiness allowlists Memory"
grep -Eq 'memory:healthy' "${checker}" ||
  fail "Memory is ready only after its health check succeeds"
grep -Eq 'known_services=.*memory' "${restore_script}" ||
  fail "restore fail-closed service inventory allowlists Memory"
grep -Eq 'known_writer_services=.*memory' "${restore_script}" ||
  fail "restore stops Memory before destructive workspace mutation"
grep -Fq 'buzz-memory-vault:/source:ro' "${backup_script}" ||
  fail "backup captures the canonical Memory authority"
grep -Fq 'memory-vault.tar.gz.enc' "${backup_script}" ||
  fail "backup encrypts the canonical Memory authority"
grep -Fq 'buzz-memory-vault:/target' "${restore_script}" ||
  fail "restore replaces the canonical Memory authority"

printf 'command memory service contract passed\n'
