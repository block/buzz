#!/usr/bin/env bash
set -euo pipefail

: "${PR_HEAD_SHA:?}"
: "${MERGE_SHA:?}"
: "${MERGED_BY:?}"
: "${VERSION:?}"
: "${PR_NUMBER:?}"
: "${GH_TOKEN:?}"

# This ID is the release-authority policy anchor. A bypass of another ruleset
# must never authorize a desktop release.
readonly DEFAULT_RULESET_ID=13596885
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
  "Desktop Release Candidate"
)

expected_branch="version-bump/$VERSION"
[[ "${PR_HEAD_REF:-}" == "$expected_branch" ]] || { echo "unexpected release branch" >&2; exit 1; }
[[ "${PR_BASE_REF:-}" == main ]] || { echo "desktop release must target main" >&2; exit 1; }
[[ "${PR_HEAD_REPO:-}" == "$GITHUB_REPOSITORY" ]] || { echo "desktop release must be internal" >&2; exit 1; }

git fetch origin "$MERGE_SHA" "$PR_HEAD_SHA" refs/heads/main:refs/remotes/origin/main --no-tags
mapfile -t parents < <(git show -s --format='%P' "$MERGE_SHA" | tr ' ' '\n')
[[ "${#parents[@]}" -eq 1 ]] || { echo "desktop release was not squash merged" >&2; exit 1; }
base_sha="$(git show "$PR_HEAD_SHA:.release/desktop-candidate.json" | jq -r .base_sha)"
[[ "${parents[0]}" == "$base_sha" ]] || { echo "squash parent is not the frozen candidate base" >&2; exit 1; }
[[ "$(git show -s --format=%T "$MERGE_SHA")" == "$(git show -s --format=%T "$PR_HEAD_SHA")" ]] || {
  echo "squash tree differs from the validated candidate" >&2
  exit 1
}
git merge-base --is-ancestor "$MERGE_SHA" origin/main || { echo "squash commit is not reachable from current main" >&2; exit 1; }

git checkout --detach "$PR_HEAD_SHA"
scripts/desktop_release.py validate --candidate "$PR_HEAD_SHA" --version "$VERSION" --repo "$GITHUB_REPOSITORY"

review="$(gh api graphql -f query='query($owner:String!,$repo:String!,$number:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$number){reviewDecision}}}' -F owner="${GITHUB_REPOSITORY%/*}" -F repo="${GITHUB_REPOSITORY#*/}" -F number="$PR_NUMBER" --jq '.data.repository.pullRequest')"
reviews="$(gh api --paginate --slurp "repos/$GITHUB_REPOSITORY/pulls/$PR_NUMBER/reviews?per_page=100")"
valid_approvals="$(jq --arg sha "$PR_HEAD_SHA" '[.[][] | select(.state == "APPROVED" and .commit_id == $sha and (.author_association == "MEMBER" or .author_association == "OWNER" or .author_association == "COLLABORATOR"))] | length' <<<"$reviews")"
review_authorized=false
if jq -e -f scripts/review-decision-approved.jq <<<"$review" >/dev/null && [[ "$valid_approvals" -gt 0 ]]; then
  review_authorized=true
fi

# Rule suites are GitHub's durable record that a permitted bypass actor landed
# this exact main update. The suite does not identify the matching bypass grant,
# so the Default ruleset's bypass list is itself the release-authority policy.
bypass_authorized=false
for attempt in {1..5}; do
  suites="$(gh api --paginate --slurp "repos/$GITHUB_REPOSITORY/rulesets/rule-suites?ref=refs/heads/main&per_page=100")"
  mapfile -t suite_ids < <(jq -r --arg before "$base_sha" --arg after "$MERGE_SHA" --arg actor "$MERGED_BY" '
    .[][] | select(.ref == "refs/heads/main" and .before_sha == $before and .after_sha == $after and .actor_name == $actor and .result == "bypass") | .id
  ' <<<"$suites")
  if [[ "${#suite_ids[@]}" -gt 1 ]]; then
    echo "multiple rule suites matched the release landing" >&2
    exit 1
  fi
  if [[ "${#suite_ids[@]}" -eq 1 ]]; then
    suite="$(gh api "repos/$GITHUB_REPOSITORY/rulesets/rule-suites/${suite_ids[0]}")"
    if jq -e --argjson ruleset_id "$DEFAULT_RULESET_ID" -f scripts/desktop-release-bypass-authorized.jq <<<"$suite" >/dev/null; then
      bypass_authorized=true
    fi
    break
  fi
  [[ "$attempt" -eq 5 ]] || sleep "$attempt"
done

[[ "$review_authorized" == true || "$bypass_authorized" == true ]] || {
  echo "release lacks an exact-head approval or authorized Default-ruleset bypass" >&2
  exit 1
}

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

echo "verified desktop candidate $PR_HEAD_SHA at squash $MERGE_SHA"
