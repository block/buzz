#!/usr/bin/env bash
# Reject organization-only package hosts in committed lockfiles so external
# contributors can `uv sync --frozen` without Block Artifactory credentials
# (block/buzz#2226).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRIVATE_HOST_PATTERN='global\.block-artifacts\.com|artifactory\.|/block-pypi/'

LOCKFILES=(
  "$ROOT/benchmarks/harbor-buzz-orchestra/uv.lock"
  "$ROOT/benchmarks/harbor-buzz-orchestra/testbed/uv.lock"
)

failed=0
for lockfile in "${LOCKFILES[@]}"; do
  if [[ ! -f "$lockfile" ]]; then
    echo "error: missing lockfile: $lockfile" >&2
    failed=1
    continue
  fi
  if grep -nE "$PRIVATE_HOST_PATTERN" "$lockfile" >&2; then
    echo "error: $lockfile references a private package host (see matches above)." >&2
    echo "Regenerate with public PyPI, e.g.:" >&2
    echo "  uv lock --index-url https://pypi.org/simple" >&2
    failed=1
  fi
done

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi

echo "ok: harbor uv.lock files use public package indexes only"
