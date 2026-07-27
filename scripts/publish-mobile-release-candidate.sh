#!/usr/bin/env bash
set -euo pipefail

fail() {
  echo "Error: $*" >&2
  exit 1
}

# shellcheck source=scripts/release-rulesets.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-rulesets.sh"

[[ "$#" -eq 3 ]] || fail "usage: $0 X.Y.Z N COMMIT_SHA"
version="$1"
candidate_number="$2"
target_sha="$3"
repo="${GITHUB_REPOSITORY:-}"

[[ "$repo" == "block/buzz" ]] || fail "candidate publishing is restricted to block/buzz"
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || \
  fail "'$version' is not a mobile release version (expected X.Y.Z)"
[[ "$candidate_number" =~ ^[1-9][0-9]*$ ]] || \
  fail "'$candidate_number' is not a candidate number (expected N >= 1)"
[[ "$target_sha" =~ ^[0-9a-f]{40}$ ]] || fail "'$target_sha' is not a full commit SHA"
command -v gh >/dev/null 2>&1 || fail "gh is required"
require_release_tag_ruleset || exit 1

main_sha="$(gh api "repos/$repo/git/ref/heads/main" --jq .object.sha)" || \
  fail "could not resolve $repo main"
[[ "$main_sha" == "$target_sha" ]] || \
  fail "$repo main moved from requested commit $target_sha to $main_sha"
commit_sha="$(gh api "repos/$repo/commits/$target_sha" --jq .sha)" || \
  fail "$target_sha is not a commit in $repo"
[[ "$commit_sha" == "$target_sha" ]] || \
  fail "GitHub resolved requested commit $target_sha as $commit_sha"

refs="$(
  gh api --paginate "repos/$repo/git/matching-refs/tags/mobile-v${version}-rc." --jq '.[].ref'
)" || fail "could not list existing candidates for $version"
next=1
while IFS= read -r ref; do
  [[ -n "$ref" ]] || continue
  [[ "$ref" =~ ^refs/tags/mobile-v${version//./\.}-rc\.([1-9][0-9]*)$ ]] || continue
  number="${BASH_REMATCH[1]}"
  (( number >= next )) && next=$((number + 1))
done <<< "$refs"
[[ "$candidate_number" -eq "$next" ]] || \
  fail "candidate sequence changed; expected rc.$candidate_number but next is rc.$next"

tag="mobile-v${version}-rc.${candidate_number}"
message="Buzz Mobile $version release candidate $candidate_number"
tag_object_sha="$(
  gh api --method POST "repos/$repo/git/tags" \
    -f tag="$tag" \
    -f message="$message" \
    -f object="$target_sha" \
    -f type=commit \
    --jq .sha
)" || fail "could not create annotated tag object for $tag"
[[ "$tag_object_sha" =~ ^[0-9a-f]{40}$ ]] || fail "GitHub returned an invalid tag object SHA"

gh api --method POST "repos/$repo/git/refs" \
  -f ref="refs/tags/$tag" \
  -f sha="$tag_object_sha" \
  --silent || fail "could not publish $tag"

published_type="$(gh api "repos/$repo/git/ref/tags/$tag" --jq .object.type)" || \
  fail "could not verify published tag $tag"
published_object="$(gh api "repos/$repo/git/ref/tags/$tag" --jq .object.sha)" || \
  fail "could not verify published tag $tag"
[[ "$published_type" == "tag" && "$published_object" == "$tag_object_sha" ]] || \
  fail "$tag does not reference the expected annotated tag object"

direct_type="$(gh api "repos/$repo/git/tags/$tag_object_sha" --jq .object.type)" || \
  fail "could not verify annotated tag object $tag_object_sha"
direct_sha="$(gh api "repos/$repo/git/tags/$tag_object_sha" --jq .object.sha)" || \
  fail "could not verify annotated tag object $tag_object_sha"
[[ "$direct_type" == "commit" && "$direct_sha" == "$target_sha" ]] || \
  fail "$tag does not point directly to requested commit $target_sha"

printf 'Published %s at %s through buzz-release-bot.\n' "$tag" "$target_sha"
