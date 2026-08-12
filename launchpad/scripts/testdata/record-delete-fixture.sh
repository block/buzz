#!/usr/bin/env bash
# Manufacture the one fixture that has no real source: a pull request that
# DELETES the nearest rules file for a path it also touches.
#
# Why this exists. Nearest-rules resolution must read the PR's head tree, not the
# local worktree — and the direction that proves it is deletion. No PR in this
# fork's history deletes a rules file, and none was found upstream, so the add
# case (PR 14, real) would otherwise have to stand in for both by assertion. It
# does not: a resolver that reads the local checkout passes the add case and
# fails this one.
#
# What it does to the world: pushes a branch, opens a PR against launchpad,
# records two responses, then closes the PR unmerged and deletes the branch.
# Nothing is merged. The PR number is left in delete-pr.number so record.sh can
# read the responses without ever opening a PR itself.
#
# Usage: launchpad/scripts/testdata/record-delete-fixture.sh
set -euo pipefail

cd "$(dirname "$0")"
HERE=$(pwd)
REPO=launchpad-26/buzz
BRANCH=fixture/rules-file-delete-throwaway
WORK=$(mktemp -d)

cleanup() {
  git -C "$HERE" worktree remove --force "$WORK" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

git -C "$HERE" fetch origin launchpad
git -C "$HERE" worktree add -f -b "$BRANCH" "$WORK" origin/launchpad

# Delete the nearest rules file for launchpad/, and touch a path it governs, so
# the deleted file and an affected path are in the same diff.
git -C "$WORK" rm -q launchpad/AGENTS.md
printf '\n<!-- throwaway fixture branch; never merged -->\n' >> "$WORK/launchpad/README.md"
git -C "$WORK" add launchpad/README.md

# --no-verify: the pre-commit hook runs `just desktop-tauri-fmt`, which fails in
# any git worktree (cargo fmt resolves workspace paths from the worktree root).
# This commit exists to produce an API response and is deleted minutes later.
git -C "$WORK" commit -s --no-verify -q -m 'chore(fixtures): throwaway — delete a rules file to record the API response

Not for review and not for merge. Opened to record the DELETE direction of the
nearest-rules fixture for the pre-flight in launchpad-26/buzz#116, because no
real pull request in this fork or upstream deletes a rules file. Closed
unmerged as soon as the response is recorded.'

git -C "$WORK" push -q --set-upstream origin "$BRANCH"

gh pr create --repo "$REPO" --base launchpad --head "$BRANCH" \
  --title 'chore(fixtures): throwaway — recording a rules-file-DELETE response (closing immediately)' \
  --body 'Do not review. Do not merge. Closing as soon as two API responses are recorded.

This PR exists only so that launchpad-26/buzz#116 can record what the compare
and tree endpoints return for a pull request that **deletes** the nearest rules
file for a path it also touches. No PR in this fork'"'"'s history does that, and
none was found upstream, so the add direction (PR #14, real) would otherwise
have had to stand in for both by assertion.

Refs #116' > /dev/null

DEL=$(gh pr list --repo "$REPO" --head "$BRANCH" --json number -q '.[0].number')
[ -n "$DEL" ] || { echo "could not read the PR number back" >&2; exit 1; }
echo "$DEL" > delete-pr.number
echo "opened throwaway PR #$DEL"

DEL_HEAD=$(gh pr view "$DEL" --repo "$REPO" --json headRefOid -q .headRefOid)
gh api "repos/$REPO/compare/launchpad...$DEL_HEAD" \
  | jq '{base_commit: {sha: .base_commit.sha}, merge_base_commit: {sha: .merge_base_commit.sha},
         status, files: [.files[] | {filename, status, additions, deletions}]}' > prdelete-compare.json
gh api "repos/$REPO/git/trees/$DEL_HEAD?recursive=1" \
  | jq '{sha, url, truncated, tree: [.tree[] | select(.path | endswith(".md"))]}' > prdelete-tree.json

gh pr close "$DEL" --repo "$REPO" --delete-branch \
  --comment 'Recorded. Closing unmerged — this PR was only ever a fixture source for #116.'
echo "closed PR #$DEL and deleted $BRANCH"

python3 - <<'PY'
import json
c = json.load(open('prdelete-compare.json'))
t = json.load(open('prdelete-tree.json'))
deleted = [f['filename'] for f in c['files'] if f['status'] == 'removed']
paths = {e['path'] for e in t['tree']}
assert 'launchpad/AGENTS.md' in deleted, f'expected a rules-file deletion, got {deleted}'
assert 'launchpad/AGENTS.md' not in paths, 'head tree still lists the deleted file'
assert 'AGENTS.md' in paths, 'root AGENTS.md should still be there to fall back to'
print('verified: launchpad/AGENTS.md deleted, absent from the head tree, root AGENTS.md intact')
PY
