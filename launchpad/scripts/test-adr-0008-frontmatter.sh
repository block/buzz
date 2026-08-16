#!/usr/bin/env bash
# Verifies ADR-0008's frontmatter and numbering are consistent with the
# repo's own rule (sequential by acceptance order, never issue order) and
# with the sibling ADRs already in flight (0005 via PR #152, 0006/0007 via
# PR #161) so this doesn't repeat the numbering mistake found in #161's
# review.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DECISIONS_DIR="${SCRIPT_DIR}/../decisions"
FILE="${DECISIONS_DIR}/ADR-0008-security-audit-privilege.md"

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
check "ADR-0008 exists at the expected path" "${status}"

status=1
[ ! -f "${DECISIONS_DIR}/ADR-0065-security-audit-privilege.md" ] && status=0
check "not filed under the issue number (0065) — that mistake is what #161's review caught" "${status}"

status=1
python3 -c "
import yaml
text = open('${FILE}').read()
front = text.split('---')[1]
data = yaml.safe_load(front)
assert data['status'] == 'Proposed', data['status']
assert data['issue'] == 'launchpad-26/buzz#65', data['issue']
assert data['decided_in'] == 'launchpad-26/buzz#65', data['decided_in']
assert data['supersedes'] == 'none', data['supersedes']
" && status=0
check "frontmatter parses and status is Proposed (not Accepted) while the PR is open" "${status}"

status=1
grep -q "^# ADR-0008 —" "${FILE}" && status=0
check "the H1 heading number matches the filename" "${status}"

# No two ADR files may claim the same number, across everything in the tree
# (0001-0004 merged; 0005, 0006, 0007 exist only on other open branches, not
# here, so this only guards against a collision within this branch itself).
status=1
dupes=$(ls "${DECISIONS_DIR}" | grep -oE '^ADR-[0-9]{4}' | sort | uniq -d)
[ -z "${dupes}" ] && status=0
check "no duplicate ADR numbers on this branch (found: ${dupes:-none})" "${status}"

echo ""
echo "Full decisions/ directory listing (verify no duplicate numbers by eye):"
ls "${DECISIONS_DIR}"

echo ""
echo "==================================================="
echo "${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
