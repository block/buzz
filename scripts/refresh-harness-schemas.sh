#!/usr/bin/env bash
# Refresh checked-in JSON Schema snapshots for codex and claude-code.
# Re-fetches both schemas, recomputes SHA-256 hashes, updates versions.json,
# and prints a diff summary.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCHEMAS_DIR="${REPO_ROOT}/desktop/src-tauri/src/managed_agents/config_bridge/schemas"
VERSIONS_FILE="${SCHEMAS_DIR}/versions.json"

CODEX_URL="https://raw.githubusercontent.com/openai/codex/main/codex-rs/core/config.schema.json"
CLAUDE_URL="https://www.schemastore.org/claude-code-settings.json"

CODEX_FILE="${SCHEMAS_DIR}/codex.config.schema.json"
CLAUDE_FILE="${SCHEMAS_DIR}/claude-code-settings.schema.json"

echo "==> Fetching codex schema..."
curl -fsSL "${CODEX_URL}" -o "${CODEX_FILE}"

echo "==> Fetching claude-code schema..."
curl -fsSL "${CLAUDE_URL}" -o "${CLAUDE_FILE}"

FETCHED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
CODEX_SHA="$(shasum -a 256 "${CODEX_FILE}" | awk '{print $1}')"
CLAUDE_SHA="$(shasum -a 256 "${CLAUDE_FILE}" | awk '{print $1}')"

# Read previous SHAs for change detection (may be absent on first run)
OLD_CODEX_SHA="$(python3 -c "import json,sys; d=json.load(open('${VERSIONS_FILE}')); print(d['codex']['sha256'])" 2>/dev/null || echo '')"
OLD_CLAUDE_SHA="$(python3 -c "import json,sys; d=json.load(open('${VERSIONS_FILE}')); print(d['claude']['sha256'])" 2>/dev/null || echo '')"

cat > "${VERSIONS_FILE}" << JSON
{
  "codex": {
    "source_url": "${CODEX_URL}",
    "fetched_at": "${FETCHED_AT}",
    "sha256": "${CODEX_SHA}"
  },
  "claude": {
    "source_url": "${CLAUDE_URL}",
    "fetched_at": "${FETCHED_AT}",
    "sha256": "${CLAUDE_SHA}"
  }
}
JSON

if [ -n "${OLD_CODEX_SHA}" ] && [ "${OLD_CODEX_SHA}" != "${CODEX_SHA}" ]; then
  echo "⚠️  codex schema changed: ${OLD_CODEX_SHA} → ${CODEX_SHA}"
fi
if [ -n "${OLD_CLAUDE_SHA}" ] && [ "${OLD_CLAUDE_SHA}" != "${CLAUDE_SHA}" ]; then
  echo "⚠️  claude schema changed: ${OLD_CLAUDE_SHA} → ${CLAUDE_SHA}"
fi

echo ""
echo "==> Updated versions.json:"
cat "${VERSIONS_FILE}"

echo ""
echo "==> Git diff summary:"
git -C "${REPO_ROOT}" diff --stat -- "${SCHEMAS_DIR}" || true
