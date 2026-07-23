#!/usr/bin/env bash
# Fail if a PR edits a migration that already exists on the base branch.
#
# sqlx stores a checksum per applied migration. Editing a shipped migration
# in place makes every long-lived database refuse to start on upgrade
# ("migration N was previously applied but has been modified") — see #2472.
# New migration files are fine; only content changes to pre-existing paths fail.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ "${GITHUB_EVENT_NAME:-}" == "pull_request" ]]; then
  BASE_REF="${GITHUB_BASE_REF:-main}"
  git fetch --depth=1 origin "$BASE_REF" 2>/dev/null || true
  BASE="origin/${BASE_REF}"
else
  # Local / push: compare against the previous commit when available.
  BASE="${MIGRATION_IMMUTABILITY_BASE:-HEAD^}"
fi

if ! git rev-parse --verify "$BASE" >/dev/null 2>&1; then
  echo "check-migration-immutability: base '$BASE' unavailable; skipping"
  exit 0
fi

# Paths under migrations/ that exist on BASE and differ at HEAD.
# Two-dot diff stays reliable on shallow CI checkouts (three-dot needs a merge base).
changed="$(git diff --name-only --diff-filter=M "$BASE" HEAD -- migrations/ || true)"

if [[ -z "${changed}" ]]; then
  echo "check-migration-immutability: no pre-existing migration files modified"
  exit 0
fi

echo "check-migration-immutability: refusing in-place edits to shipped migrations:" >&2
echo "${changed}" | sed 's/^/  /' >&2
echo "Add a new migration instead (append-only). See #2472." >&2
exit 1
