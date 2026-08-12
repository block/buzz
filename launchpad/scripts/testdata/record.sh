#!/usr/bin/env bash
# Record every pre-flight fixture from the live API.
#
# Fixtures are RECORDED, never hand-written: a fixture someone typed proves only
# that the code agrees with the author's belief about the response shape. Run
# this to re-record after an API change, and read the diff.
#
# Two fixtures are projections rather than whole responses, because the whole
# response is megabytes of noise. Each projection keeps every key of every entry
# it keeps — the shape is untouched, only irrelevant entries are dropped, and the
# jq filter that did it is right here:
#   pr86-tree.json        4337 entries -> the 120 *.md entries    (1.1 MB -> 30 KB)
#   tree-truncated.json   71798 entries -> the first 20 entries   (11 MB -> 4 KB)
# Nothing in the pre-flight reads a non-markdown tree entry, and the truncated
# fixture exists for its `truncated: true` flag alone.
#
# Usage: launchpad/scripts/testdata/record.sh
set -euo pipefail

cd "$(dirname "$0")"
REPO=launchpad-26/buzz
UP=block/buzz

say() { printf '  %-34s %s\n' "$1" "$2"; }

# ---------------------------------------------------------------- fixture (i)
# PR 86 — the everything-is-readable case. Its check list is the reason the
# record carries checks as a list: three of its checks share the name "check",
# so any name-keyed map silently drops two of them.
PR86_HEAD=$(gh pr view 86 --repo "$REPO" --json headRefOid -q .headRefOid)
PR86_BASE=$(gh pr view 86 --repo "$REPO" --json baseRefName -q .baseRefName)

gh api "repos/$REPO/pulls/86" > pr86-pr.json
say pr86-pr.json "the PR itself (identity: base ref, head sha)"

gh pr view 86 --repo "$REPO" --json title,body,labels > pr86-meta.json
say pr86-meta.json "title, body, labels"

# isRequired is GraphQL-only — it is not a field on `gh pr view --json
# statusCheckRollup`, whose keys are __typename, completedAt, conclusion,
# detailsUrl, name, startedAt, status, workflowName.
gh api graphql -f owner="${REPO%%/*}" -f repo="${REPO##*/}" -F pr=86 -f query='
query($owner:String!,$repo:String!,$pr:Int!){
  repository(owner:$owner,name:$repo){
    pullRequest(number:$pr){
      commits(last:1){nodes{commit{oid statusCheckRollup{state contexts(first:100){totalCount nodes{
        __typename
        ... on CheckRun{name status conclusion detailsUrl isRequired(pullRequestNumber:$pr) checkSuite{workflowRun{workflow{name}}}}
        ... on StatusContext{context state targetUrl isRequired(pullRequestNumber:$pr)}
      }}}}}}
    }
  }
}' > pr86-checks.json
say pr86-checks.json "the check list, with per-context isRequired"

gh api "repos/$REPO/compare/$PR86_BASE...$PR86_HEAD" > pr86-compare.json
say pr86-compare.json "merge base + per-file diff"

gh api "repos/$REPO/git/trees/$PR86_HEAD?recursive=1" \
  | jq '{sha, url, truncated, tree: [.tree[] | select(.path | endswith(".md"))]}' > pr86-tree.json
say pr86-tree.json "head tree, projected to *.md entries"

# --------------------------------------------------------------- fixture (ii)
# A closing keyword in visible body text.
gh pr view 92 --repo "$REPO" --json title,body,labels > pr92-meta.json
say pr92-meta.json "closing keyword present, in visible text"

# -------------------------------------------------------------- fixture (iii)
# The only closing keyword sits inside an HTML comment — an unfilled template
# placeholder. GitHub ignores it and so must the record.
gh pr view 5695 --repo "$UP" --json title,body,labels > upstream5695-meta.json
say upstream5695-meta.json "closing keyword ONLY inside <!-- -->"

# --------------------------------------------------------------- fixture (iv)
# A PR number that does not exist. `gh api` prints the error body on stdout and
# exits 1; both halves are the fixture, so the exit code is recorded beside it.
set +e
gh api "repos/$REPO/pulls/999999" > pr-notfound.json 2> pr-notfound.stderr
echo "$?" > pr-notfound.exit
set -e
say pr-notfound.json "404 body, exit code recorded alongside"

# ---------------------------------------------------------------- fixture (v)
# Repo-level rules for the base branch: readable, and empty. An empty list is a
# fact ("no rules configured"), not a failure, and must not read as either a
# missing gate or a scope error.
gh api "repos/$REPO/rules/branches/launchpad" > rules-branches-launchpad.json
say rules-branches-launchpad.json "repo rules for launchpad — readable, empty"

# Org-level rulesets need admin:org, which this token does not hold (gist,
# project, read:org, repo, workflow). Forbidden is a different fact from empty,
# and this is the fixture that proves the record says so.
set +e
gh api "orgs/${REPO%%/*}/rulesets" > orgs-rulesets-forbidden.json 2> orgs-rulesets-forbidden.stderr
echo "$?" > orgs-rulesets-forbidden.exit
set -e
say orgs-rulesets-forbidden.json "org rulesets — forbidden, not empty"

# --------------------------------------------------------------- fixture (vi)
# A DIVERGENT base: the base branch tip is AHEAD of the commit the head branch
# forked from. Without this, no control can tell a three-dot diff from a
# two-dot one — every open PR in our fork has base tip == merge base.
#
# Recorded from whichever upstream PR is divergent TODAY. The SHAs are not
# pinned here on purpose: an upstream PR merges and the property evaporates.
DIVERGENT=""
for n in $(gh pr list --repo "$UP" --state open --limit 40 --json number -q '.[].number'); do
  base_oid=$(gh pr view "$n" --repo "$UP" --json baseRefOid -q .baseRefOid)
  head_oid=$(gh pr view "$n" --repo "$UP" --json headRefOid -q .headRefOid)
  base_ref=$(gh pr view "$n" --repo "$UP" --json baseRefName -q .baseRefName)
  mb=$(gh api "repos/$UP/compare/$base_ref...$head_oid" -q .merge_base_commit.sha 2>/dev/null) || continue
  if [ -n "$mb" ] && [ "$mb" != "$base_oid" ]; then
    DIVERGENT="$n"
    gh api "repos/$UP/pulls/$n" > upstream-divergent-pr.json
    gh api "repos/$UP/compare/$base_ref...$head_oid" \
      | jq '{base_commit, merge_base_commit, status, ahead_by, behind_by, total_commits,
             files: [.files[] | {sha, filename, status, additions, deletions, changes}]}' \
      > upstream-divergent-compare.json
    break
  fi
done
[ -n "$DIVERGENT" ] || { echo "no divergent-base PR found upstream — fixture (vi) NOT recorded" >&2; exit 1; }
say upstream-divergent-pr.json "PR $DIVERGENT: baseRefOid != merge base"
say upstream-divergent-compare.json "its compare, files projected"

# ------------------------------------------------------- fixture (vii), ADD
# PR 14 adds AGENTS.md and launchpad/AGENTS.md. Real, merged, in this fork.
PR14_HEAD=$(gh pr view 14 --repo "$REPO" --json headRefOid -q .headRefOid)
PR14_BASE=$(gh pr view 14 --repo "$REPO" --json baseRefName -q .baseRefName)
gh api "repos/$REPO/compare/$PR14_BASE...$PR14_HEAD" \
  | jq '{base_commit: {sha: .base_commit.sha}, merge_base_commit: {sha: .merge_base_commit.sha},
         status, files: [.files[] | {filename, status, additions, deletions}]}' > pr14-compare.json
gh api "repos/$REPO/git/trees/$PR14_HEAD?recursive=1" \
  | jq '{sha, url, truncated, tree: [.tree[] | select(.path | endswith(".md"))]}' > pr14-tree.json
say pr14-compare.json "rules-file ADD — real, PR 14"
say pr14-tree.json "its head tree, *.md entries"

# ------------------------------------------------------ fixture (vii), DELETE
# No PR in this fork's history and none found upstream deletes a nearest rules
# file, so this half is recorded from a throwaway PR opened for the purpose and
# closed unmerged. PR number is read from delete-pr.number, written by
# record-delete-fixture.sh, so this script never opens a PR as a side effect.
if [ -f delete-pr.number ]; then
  DEL=$(cat delete-pr.number)
  DEL_HEAD=$(gh pr view "$DEL" --repo "$REPO" --json headRefOid -q .headRefOid)
  DEL_BASE=$(gh pr view "$DEL" --repo "$REPO" --json baseRefName -q .baseRefName)
  gh api "repos/$REPO/compare/$DEL_BASE...$DEL_HEAD" \
    | jq '{base_commit: {sha: .base_commit.sha}, merge_base_commit: {sha: .merge_base_commit.sha},
           status, files: [.files[] | {filename, status, additions, deletions}]}' > prdelete-compare.json
  gh api "repos/$REPO/git/trees/$DEL_HEAD?recursive=1" \
    | jq '{sha, url, truncated, tree: [.tree[] | select(.path | endswith(".md"))]}' > prdelete-tree.json
  say prdelete-compare.json "rules-file DELETE — throwaway PR $DEL"
  say prdelete-tree.json "its head tree, launchpad/AGENTS.md absent"
else
  echo "  delete-pr.number absent — run record-delete-fixture.sh first" >&2
fi

# ------------------------------------------------------------ truncated tree
# The trees API answers HTTP 200 with a PARTIAL list once a tree passes roughly
# 100,000 entries or 7 MB. It does not fail. A path whose rules file sits beyond
# the boundary then looks exactly like "this path has no rules file" while the
# real cause is an incomplete read — which is why the pre-flight treats
# truncated: true as a failure. This fork is nowhere near the limit, so the
# fixture is recorded from a repository that is.
gh api 'repos/torvalds/linux/git/trees/master?recursive=1' \
  | jq '{sha, url, truncated, tree: [.tree[:20][]]}' > tree-truncated.json
say tree-truncated.json "truncated: true, entries projected to 20"

echo
echo "recorded $(ls -1 ./*.json | wc -l) json fixtures"
