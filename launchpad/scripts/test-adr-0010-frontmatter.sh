#!/usr/bin/env bash
# Verifies ADR-0010's frontmatter and numbering, following the same checks
# used for ADR-0008/0009.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DECISIONS_DIR="${SCRIPT_DIR}/../decisions"
FILE="${DECISIONS_DIR}/ADR-0010-upstream-intel-handbook-staleness-link.md"

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
[ -f "${FILE}" ] && status=0
check "ADR-0010 exists at the expected path" "${status}"

status=1
[ ! -f "${DECISIONS_DIR}/ADR-0059-upstream-intel-handbook-staleness-link.md" ] && status=0
check "not filed under the issue number (0059)" "${status}"

status=1
python3 -c "
import yaml
text = open('${FILE}').read()
front = text.split('---')[1]
data = yaml.safe_load(front)
assert data['status'] == 'Proposed', data['status']
assert data['issue'] == 'launchpad-26/buzz#59', data['issue']
assert data['decided_in'] == 'launchpad-26/buzz#59', data['decided_in']
assert data['supersedes'] == 'none', data['supersedes']
" && status=0
check "frontmatter parses and status is Proposed" "${status}"

status=1
grep -q "^# ADR-0010 —" "${FILE}" && status=0
check "the H1 heading number matches the filename" "${status}"

status=1
dupes=$(ls "${DECISIONS_DIR}" | grep -oE '^ADR-[0-9]{4}' | sort | uniq -d)
[ -z "${dupes}" ] && status=0
check "no duplicate ADR numbers on this branch (found: ${dupes:-none})" "${status}"

status=1
grep -q "does \*\*not\*\* reproduce" "${FILE}" && status=0
check "the audience-boundary rule (what a flag may repeat) is stated in Decision" "${status}"

echo ""
echo "==================================================="
echo "${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
