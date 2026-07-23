#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
verify="${repo_root}/scripts/verify-release-ref.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

git -C "$tmp" init -q
git -C "$tmp" config user.name test
git -C "$tmp" config user.email test@example.com
echo first >"$tmp/file"
git -C "$tmp" add file
git -C "$tmp" commit -qm first
git -C "$tmp" tag v1.2.3

(
  cd "$tmp"
  GITHUB_REF=refs/tags/v1.2.3 "$verify" v 1.2.3
)

if (
  cd "$tmp"
  GITHUB_REF=refs/heads/main "$verify" v 1.2.3
); then
  echo "branch-backed desktop release was accepted" >&2
  exit 1
fi

echo second >>"$tmp/file"
git -C "$tmp" commit -qam second
if (
  cd "$tmp"
  GITHUB_REF=refs/tags/v1.2.3 "$verify" v 1.2.3
); then
  echo "release accepted HEAD after the tag commit" >&2
  exit 1
fi

git -C "$tmp" tag relay-v2.0.0
(
  cd "$tmp"
  GITHUB_REF=refs/tags/relay-v2.0.0 "$verify" relay-v 2.0.0
)

if grep -q 'inputs\.ref' \
  "$repo_root/.github/workflows/release.yml" \
  "$repo_root/.github/workflows/docker.yml"; then
  echo "publisher workflow still accepts a caller-selected source ref" >&2
  exit 1
fi

grep -q 'verify-release-ref\.sh' "$repo_root/.github/workflows/release.yml"
grep -q 'verify-release-ref\.sh' "$repo_root/.github/workflows/docker.yml"
grep -q 'test-release-ref-contract\.sh' "$repo_root/.github/workflows/ci.yml"
"$repo_root/scripts/test-signed-canary-contract.sh"
auto_tag="$repo_root/.github/workflows/auto-tag-on-release-pr-merge.yml"
grep -q 'actions/create-github-app-token@' "$auto_tag"
grep -q 'client-id:.*vars\.BUZZ_RELEASE_TAGGER_CLIENT_ID' "$auto_tag"
grep -q 'private-key:.*secrets\.BUZZ_RELEASE_TAGGER_PRIVATE_KEY' "$auto_tag"
grep -q 'permission-contents: write' "$auto_tag"
grep -q 'GH_TOKEN:.*steps\.release-tagger\.outputs\.token' "$auto_tag"
grep -Fq 'git/refs' "$auto_tag"
grep -Fq 'if gh api "repos/$GITHUB_REPOSITORY/git/ref/tags/$TAG" --silent 2>/dev/null; then' "$auto_tag"
if grep -F 'git/ref/tags/$TAG' "$auto_tag" | grep -Fq '|| true'; then
  echo "auto-tag ignores a failed tag lookup, so a 404 body can look like an existing tag" >&2
  exit 1
fi
if grep -q 'gh workflow run' "$auto_tag"; then
  echo "auto-tag still dispatches a publisher instead of using the tag push" >&2
  exit 1
fi

echo "release ref contract passed"
