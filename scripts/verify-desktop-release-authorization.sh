#!/usr/bin/env bash
set -euo pipefail

: "${PR_HEAD_SHA:?}"
: "${PR_NUMBER:?}"
: "${GITHUB_REPOSITORY:?}"
: "${GH_TOKEN:?}"

review="$(gh api graphql -f query='query($owner:String!,$repo:String!,$number:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$number){reviewDecision}}}' -F owner="${GITHUB_REPOSITORY%/*}" -F repo="${GITHUB_REPOSITORY#*/}" -F number="$PR_NUMBER" --jq '.data.repository.pullRequest')"
reviews="$(gh api --paginate --slurp "repos/$GITHUB_REPOSITORY/pulls/$PR_NUMBER/reviews?per_page=100&page=1")"
valid_approvals="$(jq --arg sha "$PR_HEAD_SHA" '[.[][] | select(.state == "APPROVED" and .commit_id == $sha and (.author_association == "MEMBER" or .author_association == "OWNER" or .author_association == "COLLABORATOR"))] | length' <<<"$reviews")"
if ! jq -e -f scripts/review-decision-approved.jq <<<"$review" >/dev/null || [[ "$valid_approvals" -eq 0 ]]; then
  echo "release lacks an exact-head approval" >&2
  exit 1
fi
