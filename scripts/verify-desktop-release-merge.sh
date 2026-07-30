#!/usr/bin/env bash
set -euo pipefail

: "${PR_HEAD_SHA:?}"
: "${MERGE_SHA:?}"
: "${VERSION:?}"
: "${PR_NUMBER:?}"
: "${GH_TOKEN:?}"

required_checks=(
  "Desktop E2E Integration"
  "Desktop"
  "Rust Lint"
  "Security"
  "Unit Tests"
  "Windows Rust (x86_64-pc-windows-msvc)"
  "Mobile"
  "Web"
  "Backend Integration (relay e2e)"
  "Desktop E2E Relay"
  "Relay E2E"
  "Desktop Build (macOS)"
  "DCO Check"
)

expected_branch="version-bump/$VERSION"
[[ "${PR_HEAD_REF:-}" == "$expected_branch" ]] || { echo "unexpected release branch" >&2; exit 1; }
[[ "${PR_BASE_REF:-}" == main ]] || { echo "desktop release must target main" >&2; exit 1; }
[[ "${PR_HEAD_REPO:-}" == "$GITHUB_REPOSITORY" ]] || { echo "desktop release must be internal" >&2; exit 1; }

git fetch origin "$MERGE_SHA" "$PR_HEAD_SHA" refs/heads/main:refs/remotes/origin/main --no-tags
mapfile -t parents < <(git show -s --format='%P' "$MERGE_SHA" | tr ' ' '\n')
[[ "${#parents[@]}" -eq 2 ]] || { echo "desktop release was not merged with a true merge commit" >&2; exit 1; }
[[ "${parents[1]}" == "$PR_HEAD_SHA" ]] || { echo "merge parent 2 is not the reviewed candidate" >&2; exit 1; }
git merge-base --is-ancestor "$PR_HEAD_SHA" origin/main || { echo "candidate is not reachable from current main" >&2; exit 1; }

git checkout --detach "$PR_HEAD_SHA"
scripts/desktop_release.py validate --candidate "$PR_HEAD_SHA" --version "$VERSION" --repo "$GITHUB_REPOSITORY"

review=$(gh api graphql -f query='query($owner:String!,$repo:String!,$number:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$number){reviewDecision}}}' -F owner="${GITHUB_REPOSITORY%/*}" -F repo="${GITHUB_REPOSITORY#*/}" -F number="$PR_NUMBER" --jq '.data.repository.pullRequest')
jq -e -f scripts/review-decision-approved.jq <<<"$review" >/dev/null || {
  echo "pull request effective review decision is not APPROVED" >&2
  exit 1
}
reviews="$(gh api --paginate "repos/$GITHUB_REPOSITORY/pulls/$PR_NUMBER/reviews?per_page=100")"
valid_approvals="$(jq --arg sha "$PR_HEAD_SHA" '[.[] | select(.state == "APPROVED" and .commit_id == $sha and (.author_association == "MEMBER" or .author_association == "OWNER" or .author_association == "COLLABORATOR"))] | length' <<<"$reviews")"
[[ "$valid_approvals" -gt 0 ]] || { echo "candidate lacks an exact-head approval from a repository member or collaborator" >&2; exit 1; }

checks="$(gh api --paginate --slurp "repos/$GITHUB_REPOSITORY/commits/$PR_HEAD_SHA/check-runs?per_page=100")"
for required in "${required_checks[@]}"; do
  jq -e --arg name "$required" -f scripts/required-check-succeeded.jq <<<"$checks" >/dev/null || {
    echo "required check is missing or unsuccessful: $required" >&2
    exit 1
  }
done
status="$(gh api "repos/$GITHUB_REPOSITORY/commits/$PR_HEAD_SHA/status")"
jq -e '(.total_count == 0) or (.state == "success")' <<<"$status" >/dev/null || {
  echo "candidate has a failing or pending combined commit status" >&2
  exit 1
}

echo "verified reviewed desktop candidate $PR_HEAD_SHA at merge $MERGE_SHA"
