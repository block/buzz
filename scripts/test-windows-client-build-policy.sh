#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

build16=1a3084e4d3a7a93492ac9a82ebdfffb40c2def39
legacy_sibling=2025dcdde29b4c3690c7f2766dd4d14bae08e7e9
bootstrap=83afea0a1549410a8f7effa12ceed0f7b13234e6
candidate_commit=$(git rev-parse HEAD)
candidate_tree=HEAD

expect_failure() {
  local label=$1
  shift
  if "$@" >/dev/null 2>&1; then
    printf 'FAIL: %s unexpectedly passed\n' "$label" >&2
    exit 1
  fi
  printf 'PASS: %s rejected\n' "$label"
}

expect_success() {
  local label=$1
  shift
  if ! "$@"; then
    printf 'FAIL: %s unexpectedly failed\n' "$label" >&2
    exit 1
  fi
  printf 'PASS: %s accepted\n' "$label"
}

expect_failure \
  'Build 16 alone lacks the new cumulative client contract' \
  scripts/check-windows-client-contract.sh "$build16"
expect_failure \
  'a legacy sibling line lacks the cumulative client contract' \
  scripts/check-windows-client-contract.sh "$legacy_sibling"
stress_candidate_contract() {
  local iteration
  for iteration in {1..100}; do
    scripts/check-windows-client-contract.sh "$candidate_tree" >/dev/null
  done
}

expect_success \
  'the corrected cumulative candidate retains every contract repeatedly' \
  stress_candidate_contract

expect_failure \
  'a sibling candidate is not cumulative' \
  scripts/verify-windows-client-lineage.sh \
    "$build16" "$legacy_sibling" windows
expect_failure \
  'the old canonical stream name is now forbidden' \
  scripts/verify-windows-client-lineage.sh \
    "$build16" "$build16" windows-integration
expect_success \
  'the current canonical candidate descends from the old-stream bootstrap' \
  scripts/verify-windows-client-lineage.sh \
    "$bootstrap" "$candidate_commit" windows

workflow=.github/workflows/windows-fork-integration.yml
grep -Fq 'CANONICAL_WINDOWS_BRANCH: windows' "$workflow"
grep -Fq \
  'WINDOWS_BOOTSTRAP_SHA: 83afea0a1549410a8f7effa12ceed0f7b13234e6' \
  "$workflow"
grep -Fq 'branch=${CANONICAL_WINDOWS_BRANCH}' "$workflow"
grep -Fq 'scripts/verify-windows-client-lineage.sh' "$workflow"
grep -Fq 'scripts/check-windows-client-contract.sh' "$workflow"
grep -Fq 'scripts/windows_build_manifest.py preflight' "$workflow"
grep -Fq 'scripts/windows_build_manifest.py generate' "$workflow"
grep -Fq 'scripts/windows_build_manifest.py validate' "$workflow"
grep -Fq 'path: windows-build-artifact' "$workflow"
printf 'PASS: GitHub workflow invokes lineage, retained-surface, and manifest gates\n'
