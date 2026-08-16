#!/usr/bin/env bash
# Verifies ADR-0014's frontmatter and numbering, following the same checks
# used for ADR-0008/0009/0010/0011/0012/0013.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DECISIONS_DIR="${SCRIPT_DIR}/../decisions"
FILE="${DECISIONS_DIR}/ADR-0014-host-firewall-implementation.md"

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
check "ADR-0014 exists at the expected path" "${status}"

status=1
[ ! -f "${DECISIONS_DIR}/ADR-0044-host-firewall-implementation.md" ] && status=0
check "not filed under the issue number (0044)" "${status}"

status=1
python3 -c "
import yaml
text = open('${FILE}').read()
front = text.split('---')[1]
data = yaml.safe_load(front)
assert data['status'] == 'Proposed', data['status']
assert data['issue'] == 'launchpad-26/buzz#44', data['issue']
assert data['decided_in'] == 'launchpad-26/buzz#44', data['decided_in']
assert data['supersedes'] == 'none', data['supersedes']
" && status=0
check "frontmatter parses and status is Proposed" "${status}"

status=1
grep -q "^# ADR-0014 —" "${FILE}" && status=0
check "the H1 heading number matches the filename" "${status}"

status=1
dupes=$(ls "${DECISIONS_DIR}" | grep -oE '^ADR-[0-9]{4}' | sort | uniq -d)
[ -z "${dupes}" ] && status=0
check "no duplicate ADR numbers on this branch (found: ${dupes:-none})" "${status}"

status=1
grep -q "Trigger:\* any Docker Engine upgrade" "${FILE}" && status=0
check "the Docker-chain-change contingency trigger is stated" "${status}"

echo ""
echo "==================================================="
echo "${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
