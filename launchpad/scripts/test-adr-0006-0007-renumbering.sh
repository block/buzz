#!/usr/bin/env bash
# Verifies the fixes from Serina's PR #161 review: correct sequential ADR
# numbers (0006/0007, coordinating with #152's 0005), no stale ADR-0065
# citations to a decision that doesn't exist yet, and status: Proposed
# rather than Accepted while the PR is still open.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DECISIONS_DIR="${SCRIPT_DIR}/../decisions"

PASS=0
FAIL=0

check() {
  local label=$1 ok=$2
  if [ "${ok}" = "0" ]; then
    echo "PASS: ${label}"
    PASS=$((PASS + 1))
  else
    echo "FAIL: ${label}"
    FAIL=$((FAIL + 1))
  fi
}

status=1
[ -f "${DECISIONS_DIR}/ADR-0006-secret-scanning-engine-and-allowlist-location.md" ] && status=0
check "ADR-0006 (secret scanning) exists at the correct sequential number" "${status}"

status=1
[ -f "${DECISIONS_DIR}/ADR-0007-dependency-update-path.md" ] && status=0
check "ADR-0007 (dependency update path) exists at the correct sequential number" "${status}"

status=1
[ ! -f "${DECISIONS_DIR}/ADR-0063-secret-scanning-engine-and-allowlist-location.md" ] \
  && [ ! -f "${DECISIONS_DIR}/ADR-0064-dependency-update-path.md" ] && status=0
check "the old issue-numbered filenames (0063/0064) are gone" "${status}"

status=0
grep -rq "ADR-0063\|ADR-0064\|ADR-0065" "${DECISIONS_DIR}/ADR-0006-secret-scanning-engine-and-allowlist-location.md" \
  "${DECISIONS_DIR}/ADR-0007-dependency-update-path.md" && status=1
check "no stale ADR-0063/0064/0065 references remain in either file" "${status}"

status=1
grep -q "^status: Proposed$" "${DECISIONS_DIR}/ADR-0006-secret-scanning-engine-and-allowlist-location.md" && status=0
check "ADR-0006 status is Proposed, not Accepted, while the PR is open" "${status}"

status=1
grep -q "^status: Proposed$" "${DECISIONS_DIR}/ADR-0007-dependency-update-path.md" && status=0
check "ADR-0007 status is Proposed, not Accepted, while the PR is open" "${status}"

echo ""
echo "==================================================="
echo "${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
