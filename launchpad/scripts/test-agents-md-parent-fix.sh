#!/usr/bin/env bash
# Verifies #85's fix to launchpad/AGENTS.md: the nonexistent `gh issue create
# --parent` flag is gone from every command example, and the replacement
# `link_child` shell function it was replaced with is syntactically valid bash.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENTS_MD="${SCRIPT_DIR}/../AGENTS.md"

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

# --- no command example still passes --parent to gh -----------------------
# The two remaining mentions of "--parent" are prose explaining it does NOT
# exist; neither may appear as an actual flag inside a fenced shell command.
status=0
awk '
  /^```bash/ { infence = 1; next }
  /^```$/ { infence = 0; next }
  infence && /--parent/ { print; found = 1 }
  END { exit found ? 1 : 0 }
' "${AGENTS_MD}" || status=$?
check "no fenced command example still passes --parent to gh" "${status}"

# --- the link_child replacement is valid bash -------------------------------
status=0
sed -n '/^link_child() {/,/^}/p' "${AGENTS_MD}" > /tmp/link_child_extract.sh
bash -n /tmp/link_child_extract.sh || status=$?
check "the link_child() replacement function is valid bash" "${status}"

# --- the function actually appears (extraction didn't silently match nothing)
status=1
[ -s /tmp/link_child_extract.sh ] && status=0
check "link_child() was found in AGENTS.md at all" "${status}"

echo ""
echo "==================================================="
echo "${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
