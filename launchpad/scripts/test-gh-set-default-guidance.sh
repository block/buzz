#!/usr/bin/env bash
# Verifies #136's fix: both launchpad/README.md and launchpad/AGENTS.md tell
# contributors to run `gh repo set-default launchpad-26/buzz` once per clone,
# so a gh command with no explicit --repo does not silently target block/buzz.
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

for f in README.md AGENTS.md; do
  status=1
  grep -q "gh repo set-default launchpad-26/buzz" "${LAUNCHPAD_DIR}/${f}" && status=0
  check "${f} tells contributors to run gh repo set-default" "${status}"
done

echo ""
echo "==================================================="
echo "${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
