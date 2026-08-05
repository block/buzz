#!/usr/bin/env bash
set -euo pipefail

: "${PR_HEAD_SHA:?}"
: "${MERGE_SHA:?}"
: "${MERGED_AT:?}"
: "${VERSION:?}"
: "${PR_NUMBER:?}"
: "${GH_TOKEN:?}"

# Keep this list aligned with the main ruleset. Producer IDs prevent a check
# with a copied display name from authorizing a release. Every current required
# gate is a check run; add explicit legacy-status verification before introducing
# any required context that reports only through the commit-status API.
required_checks=(
  "Desktop E2E Integration:15368"
  "Desktop:15368"
  "Rust Lint:15368"
  "Security:15368"
  "Unit Tests:15368"
  "Windows Rust (x86_64-pc-windows-msvc):15368"
  "Mobile:15368"
  "Web:15368"
  "Backend Integration (relay e2e):15368"
  "Desktop E2E Relay:15368"
  "Relay E2E:15368"
  "Desktop Build (macOS):15368"
  "DCO Check:1455659"
  "Desktop Release Candidate:15368"
)

expected_branch="version-bump/$VERSION"
[[ "${PR_HEAD_REF:-}" == "$expected_branch" ]] || { echo "unexpected release branch" >&2; exit 1; }
[[ "${PR_BASE_REF:-}" == main ]] || { echo "desktop release must target main" >&2; exit 1; }
[[ "${PR_HEAD_REPO:-}" == "$GITHUB_REPOSITORY" ]] || { echo "desktop release must be internal" >&2; exit 1; }

# The API identity must match the closed event. Branch names are mutable and are
# never used to resolve the artifact.
pr="$(gh api "repos/$GITHUB_REPOSITORY/pulls/$PR_NUMBER")"
jq -e \
  --arg head "$PR_HEAD_SHA" --arg head_ref "$PR_HEAD_REF" --arg head_repo "$PR_HEAD_REPO" \
  --arg base "$PR_BASE_REF" --arg merge "$MERGE_SHA" --arg merged_at "$MERGED_AT" \
  '.merged == true and .head.sha == $head and .head.ref == $head_ref and
   .head.repo.full_name == $head_repo and .base.ref == $base and
   .merge_commit_sha == $merge and .merged_at == $merged_at' <<<"$pr" >/dev/null || {
  echo "pull request API identity does not match the closed merge event" >&2
  exit 1
}

# Pin trusted verifier code from the candidate's frozen base, not from the
# candidate or its squash. A release PR cannot alter the code that validates it.
git fetch origin main --no-tags
git fetch origin "$PR_HEAD_SHA" --no-tags
candidate_parents="$(git show -s --format=%P "$PR_HEAD_SHA")"
[[ "$candidate_parents" =~ ^[0-9a-f]{40}$ ]] || {
  echo "desktop candidate must have exactly one parent before validation" >&2
  exit 1
}
git merge-base --is-ancestor "$candidate_parents" origin/main || {
  echo "desktop candidate base is not protected main history" >&2
  exit 1
}
verifier_dir="$(mktemp -d)"
trap 'rm -rf "$verifier_dir"' EXIT
git show "$candidate_parents:scripts/desktop_release.py" > "$verifier_dir/desktop_release.py"
git show "$candidate_parents:scripts/required-check-succeeded.jq" > "$verifier_dir/required-check-succeeded.jq"

git checkout --detach "$PR_HEAD_SHA"
DESKTOP_RELEASE_ROOT="$PWD" python3 "$verifier_dir/desktop_release.py" \
  validate --candidate "$PR_HEAD_SHA" --version "$VERSION" --repo "$GITHUB_REPOSITORY"

# `filter=latest` is deliberate: GitHub exposes no per-rerun creation time. A
# post-merge rerun replaces the visible attempt and fails closed below.
checks="$(gh api --paginate --slurp "repos/$GITHUB_REPOSITORY/commits/$PR_HEAD_SHA/check-runs?filter=latest&per_page=100")"
for entry in "${required_checks[@]}"; do
  required="${entry%:*}"
  integration_id="${entry##*:}"
  jq -e --arg name "$required" --argjson integration_id "$integration_id" \
    --arg merged_at "$MERGED_AT" \
    -f "$verifier_dir/required-check-succeeded.jq" <<<"$checks" >/dev/null || {
    echo "trusted required check was not successful at merge: $required" >&2
    exit 1
  }
done

echo "verified immutable desktop candidate $PR_HEAD_SHA authorized by merged PR $PR_NUMBER"
