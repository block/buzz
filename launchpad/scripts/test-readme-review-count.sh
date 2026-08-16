#!/usr/bin/env bash
# Verifies #162: launchpad/README.md's review-count claim matches
# launchpad/AGENTS.md's (already correct, per #123's resolution) and the
# enforced reality — two approving reviews from reviewers with write access,
# not one.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAUNCHPAD_DIR="${SCRIPT_DIR}/.."

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
grep -q "two approving reviews" "${LAUNCHPAD_DIR}/README.md" && status=0
check "README.md says two approving reviews" "${status}"

status=0
grep -qi "need one approving review" "${LAUNCHPAD_DIR}/README.md" && status=1
check "README.md no longer says one approving review" "${status}"

# Regression guard: AGENTS.md's own count (already correct since #123) must
# still agree with README.md's, not just both happen to mention a number.
status=1
grep -q "at least two approving reviews" "${LAUNCHPAD_DIR}/AGENTS.md" && status=0
check "AGENTS.md still says two — the two files agree" "${status}"

echo ""
echo "==================================================="
echo "${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
