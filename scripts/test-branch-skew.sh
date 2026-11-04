#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
guard="$repo_root/scripts/check-branch-skew.sh"
fixture_root=$(mktemp -d)
trap 'rm -rf "$fixture_root"' EXIT
upstream="$fixture_root/upstream"
client="$fixture_root/client"

# Construct synthetic history without running contributor hooks on fixture commits.
commit_fixture() {
  local repo="$1" file="$2" content="$3" tree oid
  local args=(-m fixture)
  printf '%s\n' "$content" >"$repo/$file"
  git -C "$repo" add "$file"
  tree=$(git -C "$repo" write-tree)
  if git -C "$repo" rev-parse --verify HEAD >/dev/null 2>&1; then
    args+=(-p HEAD)
  fi
  oid=$(GIT_AUTHOR_NAME=Fixture GIT_AUTHOR_EMAIL=fixture@example.com \
    GIT_COMMITTER_NAME=Fixture GIT_COMMITTER_EMAIL=fixture@example.com \
    git -C "$repo" commit-tree "$tree" "${args[@]}")
  git -C "$repo" update-ref HEAD "$oid"
}

expect_pass() {
  (cd "$client" && "$guard") >"$fixture_root/result" 2>&1 || {
    cat "$fixture_root/result" >&2
    echo "Expected pass: $1" >&2
    exit 1
  }
}

expect_fail() {
  if (cd "$client" && "$guard") >"$fixture_root/result" 2>&1; then
    echo "Expected failure: $1" >&2
    exit 1
  fi
}

git init -q -b main "$upstream"
commit_fixture "$upstream" shared base
git -C "$upstream" branch parent
git -C "$upstream" branch stable
git clone -q "$upstream" "$client"
git -C "$client" checkout -qb feature
commit_fixture "$client" shared feature
expect_pass 'ordinary branch based on current main'

commit_fixture "$upstream" shared main-advanced
expect_fail 'ordinary branch overlaps newer main'
grep -q 'behind origin/main' "$fixture_root/result"

git -C "$client" config branch.feature.gh-merge-base parent
expect_pass 'stacked branch uses parent despite overlapping main'
git -C "$upstream" checkout -q parent
commit_fixture "$upstream" other disjoint
expect_pass 'parent advances in a disjoint file'
commit_fixture "$upstream" shared parent-advanced
expect_fail 'stacked branch overlaps newer parent'
grep -q 'behind origin/parent' "$fixture_root/result"

for invalid in '' 'bad base' feature; do
  git -C "$client" config branch.feature.gh-merge-base "$invalid"
  expect_fail "invalid base '$invalid'"
  grep -q 'Invalid PR base' "$fixture_root/result"
done
git -C "$client" config branch.feature.gh-merge-base missing
expect_fail 'missing remote branch'

# A cached tracking ref must not turn an unavailable upstream into success.
git -C "$client" config branch.feature.gh-merge-base stable
expect_pass 'cached base passes while upstream is reachable'
git -C "$client" config remote.origin.url "$fixture_root/unavailable"
expect_fail 'fetch failure with cached parent'

echo 'Branch-skew regression checks passed'
