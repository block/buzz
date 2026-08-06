#!/usr/bin/env bash
# Collect N successful WAVE1_PERF measurement lines, retrying failed boots.
# Resumes from an existing success log when present.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"
# shellcheck disable=SC1091
. ./bin/activate-hermit
cd desktop

LABEL="${1:?label}"
TARGET="${2:-50}"
OUT_DIR="/tmp/buzz-perf-proof"
mkdir -p "$OUT_DIR"
OUT="$OUT_DIR/${LABEL}-success.log"
touch "$OUT"

success=$(rg -c '^WAVE1_PERF' "$OUT" 2>/dev/null || echo 0)
# rg -c prints "0" or count; ensure numeric
success=${success:-0}
attempt=0

echo "Resuming $LABEL with success=$success target=$TARGET"

while (( success < TARGET )); do
  attempt=$((attempt + 1))
  echo "=== attempt=$attempt success=$success/$TARGET ($LABEL) ==="
  rm -rf /tmp/buzz-playwright-perf-results
  set +e
  LOG=$(mktemp)
  npx playwright test --config=playwright.perf.config.ts \
    tests/e2e/typing-wave1-core.perf.ts --project=perf --reporter=line \
    >"$LOG" 2>&1
  rc=$?
  set -e
  if rg -q '^WAVE1_PERF' "$LOG"; then
    rg '^WAVE1_PERF' "$LOG" | sed "s/repeat=[0-9]*/repeat=$success/" >>"$OUT"
    success=$((success + 1))
    echo "OK success=$success"
  else
    echo "FAIL rc=$rc (no WAVE1_PERF line)"
    # Keep failures short in the collect log to avoid huge files.
    rg -n 'Error:|Timeout|exceeded|ENOSPC' "$LOG" | tail -n 15 || true
  fi
  rm -f "$LOG"
done

echo "DONE $LABEL collected=$success attempts=$attempt file=$OUT"
