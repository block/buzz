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

scripts/verify-desktop-release-authorization.sh

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
