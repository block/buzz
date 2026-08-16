#!/usr/bin/env bash
# Exercises launchpad/scripts/check-branch-skew.sh (#15) against real git repos in
# a scratch directory, covering both remote-naming conventions the issue names:
# `origin` pointing straight at the cohort repo, and `origin` as a personal fork
# with a separate `launchpad` remote for the cohort repo. Each scenario gets its
# own fresh bare "cohort repo" fixture, so scenarios cannot see each other's
# commits and no scenario's outcome depends on the order the others ran in.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUARD="${SCRIPT_DIR}/check-branch-skew.sh"

WORK=$(mktemp -d)
trap 'rm -rf "${WORK}"' EXIT

PASS=0
FAIL=0

check() {
  local label=$1 expected=$2 got=$3
  if [ "${expected}" = "${got}" ]; then
    echo "PASS: ${label}"
    PASS=$((PASS + 1))
  else
    echo "FAIL: ${label} — expected exit ${expected}, got ${got}"
    FAIL=$((FAIL + 1))
  fi
}

new_bare_cohort_repo() {
  local name=$1
  git init --bare --initial-branch=launchpad --quiet "${WORK}/${name}.git"
}

configure() {
  git -C "$1" config user.email "test@example.invalid"
  git -C "$1" config user.name "Test"
}

seed() {
  # Commits + pushes an initial AGENTS.md to $1's launchpad branch, via a
  # throwaway clone so the bare repo itself is never checked out directly.
  local bare=$1
  rm -rf "${WORK}/seed"
  git clone --quiet "${bare}" "${WORK}/seed"
  configure "${WORK}/seed"
  echo "seed" > "${WORK}/seed/AGENTS.md"
  git -C "${WORK}/seed" add AGENTS.md
  git -C "${WORK}/seed" commit --quiet -m "seed"
  git -C "${WORK}/seed" push --quiet origin HEAD:launchpad
}

run_guard() {
  (cd "$1" && bash "${GUARD}")
}

# --- convention A, current: branch is at the tip of origin/launchpad -----------
new_bare_cohort_repo cohort-a1
seed "${WORK}/cohort-a1.git"
rm -rf "${WORK}/a1"
git clone --quiet "${WORK}/cohort-a1.git" "${WORK}/a1"
configure "${WORK}/a1"
git -C "${WORK}/a1" checkout --quiet -b feature-current
status=0
run_guard "${WORK}/a1" || status=$?
check "convention A: branch current with origin/launchpad exits 0" 0 "${status}"

# --- convention A, behind, no overlap: must not block --------------------------
new_bare_cohort_repo cohort-a2
seed "${WORK}/cohort-a2.git"
rm -rf "${WORK}/a2"
git clone --quiet "${WORK}/cohort-a2.git" "${WORK}/a2"
configure "${WORK}/a2"
git -C "${WORK}/a2" checkout --quiet -b feature-behind-no-overlap
echo "unrelated" > "${WORK}/a2/other-file.txt"
git -C "${WORK}/a2" add other-file.txt
git -C "${WORK}/a2" commit --quiet -m "unrelated local change"
# Advance the cohort repo's launchpad branch AFTER the feature branch forked,
# touching a DIFFERENT file, via a second independent clone.
rm -rf "${WORK}/a2-upstream"
git clone --quiet "${WORK}/cohort-a2.git" "${WORK}/a2-upstream"
configure "${WORK}/a2-upstream"
echo "upstream change" > "${WORK}/a2-upstream/unrelated-upstream.txt"
git -C "${WORK}/a2-upstream" add unrelated-upstream.txt
git -C "${WORK}/a2-upstream" commit --quiet -m "upstream advances"
git -C "${WORK}/a2-upstream" push --quiet origin HEAD:launchpad
status=0
run_guard "${WORK}/a2" || status=$?
check "convention A: behind but no overlapping files exits 0" 0 "${status}"

# --- convention A, behind, WITH overlap: must block -----------------------------
new_bare_cohort_repo cohort-a3
seed "${WORK}/cohort-a3.git"
rm -rf "${WORK}/a3"
git clone --quiet "${WORK}/cohort-a3.git" "${WORK}/a3"
configure "${WORK}/a3"
git -C "${WORK}/a3" checkout --quiet -b feature-behind-overlap
echo "feature branch also edits this" >> "${WORK}/a3/AGENTS.md"
git -C "${WORK}/a3" commit --quiet -am "feature branch edits AGENTS.md"
rm -rf "${WORK}/a3-upstream"
git clone --quiet "${WORK}/cohort-a3.git" "${WORK}/a3-upstream"
configure "${WORK}/a3-upstream"
echo "upstream also edits this" >> "${WORK}/a3-upstream/AGENTS.md"
git -C "${WORK}/a3-upstream" commit --quiet -am "upstream edits AGENTS.md too"
git -C "${WORK}/a3-upstream" push --quiet origin HEAD:launchpad
status=0
run_guard "${WORK}/a3" || status=$?
check "convention A: behind WITH overlapping file exits 1" 1 "${status}"

# --- convention B: origin is a personal fork, `launchpad` is the cohort repo ---
new_bare_cohort_repo cohort-b
seed "${WORK}/cohort-b.git"
rm -rf "${WORK}/personal-fork.git"
git init --bare --initial-branch=launchpad --quiet "${WORK}/personal-fork.git"
rm -rf "${WORK}/b"
git clone --quiet "${WORK}/cohort-b.git" "${WORK}/b" --origin launchpad
configure "${WORK}/b"
git -C "${WORK}/b" remote add origin "${WORK}/personal-fork.git"
git -C "${WORK}/b" checkout --quiet -b feature-b
echo "feature branch edits this" >> "${WORK}/b/AGENTS.md"
git -C "${WORK}/b" commit --quiet -am "edits AGENTS.md on convention B"
rm -rf "${WORK}/b-upstream"
git clone --quiet "${WORK}/cohort-b.git" "${WORK}/b-upstream"
configure "${WORK}/b-upstream"
echo "cohort repo also edits this" >> "${WORK}/b-upstream/AGENTS.md"
git -C "${WORK}/b-upstream" commit --quiet -am "cohort repo edits AGENTS.md too"
git -C "${WORK}/b-upstream" push --quiet origin HEAD:launchpad
status=0
run_guard "${WORK}/b" || status=$?
check "convention B: launchpad remote (not origin) is found and still blocks" 1 "${status}"

# --- no remote has a `launchpad` ref at all: must not block --------------------
rm -rf "${WORK}/c.git" "${WORK}/c"
git init --bare --initial-branch=main --quiet "${WORK}/c.git"
git clone --quiet "${WORK}/c.git" "${WORK}/c"
configure "${WORK}/c"
echo "x" > "${WORK}/c/f.txt"
git -C "${WORK}/c" add f.txt
git -C "${WORK}/c" commit --quiet -m "seed"
git -C "${WORK}/c" push --quiet origin HEAD:main
git -C "${WORK}/c" checkout --quiet -b feature-c
status=0
run_guard "${WORK}/c" || status=$?
check "no remote has a launchpad ref: does not block" 0 "${status}"

echo ""
echo "==================================================="
echo "${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
