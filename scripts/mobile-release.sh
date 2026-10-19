#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage:
  scripts/mobile-release.sh candidate X.Y.Z [--source-branch mobile-vX.Y] [--commit FULL_SHA]

candidate  Publish the next immutable mobile-vX.Y.Z-rc.N candidate tag at the
           selected commit in source history (defaults to the current source tip).
USAGE
  exit 2
}

fail() {
  echo "Error: $*" >&2
  exit 1
}

# shellcheck source=scripts/release-rulesets.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-rulesets.sh"
# shellcheck source=scripts/mobile-release-source.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/mobile-release-source.sh"

require_gh_minimum_version() {
  local minimum="2.87.0" version

  command -v gh >/dev/null 2>&1 || fail "gh >= $minimum is required"
  version="$(gh --version 2>/dev/null | awk 'NR == 1 { print $3 }')" || \
    fail "could not determine gh version (gh >= $minimum is required)"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || \
    fail "gh returned invalid version '$version'"
  if ! awk -v current="$version" -v minimum="$minimum" '
    BEGIN {
      split(current, c, ".")
      split(minimum, m, ".")
      for (i = 1; i <= 3; i++) {
        if ((c[i] + 0) > (m[i] + 0)) exit 0
        if ((c[i] + 0) < (m[i] + 0)) exit 1
      }
      exit 0
    }
  '; then
    fail "gh $version is too old; gh >= $minimum is required"
  fi
}

require_clean_semver() {
  [[ "$1" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || \
    fail "'$1' is not a mobile release version (expected X.Y.Z)"
}

require_clean_tree() {
  git diff --quiet && git diff --cached --quiet && \
    [[ -z "$(git status --short --untracked-files=normal)" ]] || \
    fail "working tree is dirty; commit or stash changes first"
}

require_annotated_tag() {
  local git_dir="$1" object="$2" label="$3"
  if [[ "$(git -C "$git_dir" cat-file -t "$object" 2>/dev/null || true)" != "tag" ]]; then
    echo "$label must be an annotated tag" >&2
    return 1
  fi
}

remote_tag_commit_sha() {
  local ref="$1" line advertised_oid tmp fetched_oid commit
  line="$(git ls-remote --refs origin "$ref")" || return 1
  [[ -n "$line" && "$line" != *$'\n'* ]] || return 1
  advertised_oid="${line%%$'\t'*}"
  tmp="$(mktemp -d)"
  git -C "$tmp" init -q
  git -C "$tmp" remote add origin "$(git remote get-url origin)"
  if ! git -C "$tmp" fetch -q --depth 1 origin "$ref"; then
    rm -rf "$tmp"
    return 1
  fi
  fetched_oid="$(git -C "$tmp" rev-parse --verify FETCH_HEAD)"
  if [[ "$fetched_oid" != "$advertised_oid" ]]; then
    rm -rf "$tmp"
    fail "$ref moved while it was being resolved"
  fi
  if ! require_annotated_tag "$tmp" FETCH_HEAD "$ref"; then
    rm -rf "$tmp"
    return 1
  fi
  if ! commit="$(git -C "$tmp" rev-parse --verify 'FETCH_HEAD^{commit}')"; then
    rm -rf "$tmp"
    return 1
  fi
  rm -rf "$tmp"
  printf '%s' "$commit"
}

remote_source_commit_sha() {
  local ref="refs/heads/$MOBILE_RELEASE_SOURCE_BRANCH" fetched_oid commit
  # Fetch a single snapshot. A later branch advance does not invalidate it.
  git fetch -q --no-tags origin "$ref" || return 1
  fetched_oid="$(git rev-parse --verify FETCH_HEAD)" || return 1
  commit="$(git rev-parse --verify 'FETCH_HEAD^{commit}')" || return 1
  [[ "$commit" == "$fetched_oid" ]] || \
    fail "origin/$MOBILE_RELEASE_SOURCE_BRANCH did not resolve directly to a commit"
  printf '%s' "$commit"
}

command="${1:-}"
case "$command" in
  candidate)
    [[ "$#" -ge 2 ]] || usage
    version="$2"
    shift 2
    MOBILE_RELEASE_SOURCE_BRANCH=main
    requested_sha=""
    source_option_seen=false
    commit_option_seen=false
    while [[ "$#" -gt 0 ]]; do
      [[ "$#" -ge 2 ]] || usage
      case "$1" in
        --source-branch)
          [[ "$source_option_seen" == false ]] || usage
          source_option_seen=true
          MOBILE_RELEASE_SOURCE_BRANCH="$2"
          ;;
        --commit)
          [[ "$commit_option_seen" == false ]] || usage
          commit_option_seen=true
          requested_sha="$2"
          [[ "$requested_sha" =~ ^[0-9a-f]{40}$ ]] || fail "--commit requires a full commit SHA"
          ;;
        *) usage ;;
      esac
      shift 2
    done
    require_clean_semver "$version"
    require_mobile_release_source "$version" "$MOBILE_RELEASE_SOURCE_BRANCH" || exit 1
    require_clean_tree
    require_canonical_repository || exit 1
    require_gh_minimum_version
    require_mobile_release_branch_ruleset "$MOBILE_RELEASE_SOURCE_BRANCH" || exit 1

    local_head_sha="$(git rev-parse --verify 'HEAD^{commit}')" || fail "HEAD is not a commit"
    source_tip="$(remote_source_commit_sha)" || fail "origin/$MOBILE_RELEASE_SOURCE_BRANCH does not exist"
    source_sha="${requested_sha:-$source_tip}"
    require_mobile_release_commit_ancestor "$source_sha" "$source_tip" || exit 1
    next=1
    if ! remote_tags="$(git ls-remote --refs --tags origin "refs/tags/mobile-v${version}-rc.*")"; then
      fail "could not list existing candidates for $version"
    fi
    while IFS=$'\t' read -r _ ref; do
      [[ "$ref" =~ ^refs/tags/mobile-v${version//./\.}-rc\.([1-9][0-9]*)$ ]] || continue
      number="${BASH_REMATCH[1]}"
      (( number >= next )) && next=$((number + 1))
    done <<< "$remote_tags"

    tag="mobile-v${version}-rc.${next}"
    workflow="mobile-release-candidate.yml"
    if dispatch_output="$(gh workflow run "$workflow" \
      --repo block/buzz \
      --ref main \
      -f "version=$version" \
      -f "candidate_number=$next" \
      -f "source_branch=$MOBILE_RELEASE_SOURCE_BRANCH" \
      -f "target_sha=$source_sha" 2>&1)"; then
      :
    else
      if [[ "$dispatch_output" == *"does not have 'workflow_dispatch' trigger"* ]]; then
        fail "$workflow is not available on main yet; merge the release-process change before publishing a candidate"
      fi
      fail "could not dispatch App-backed publication for $tag: $dispatch_output"
    fi
    run_url="$(printf '%s\n' "$dispatch_output" | awk '/^https:\/\/github\.com\/block\/buzz\/actions\/runs\/[0-9]+$/ { if (found) exit 2; found = $0 } END { if (found) print found }')" || \
      fail "GitHub returned multiple workflow run URLs for one candidate dispatch"
    [[ -n "$run_url" ]] || \
      fail "GitHub accepted the candidate dispatch but returned no workflow run URL"
    run_id="${run_url##*/}"
    gh run watch "$run_id" --repo block/buzz --exit-status --compact || \
      fail "App-backed publication failed: $run_url"

    published_sha="$(remote_tag_commit_sha "refs/tags/$tag")" || \
      fail "publication completed without exact annotated candidate tag $tag"
    [[ "$published_sha" == "$source_sha" ]] || \
      fail "$tag resolved to $published_sha instead of requested commit $source_sha"
    if [[ "$local_head_sha" != "$source_sha" ]]; then
      echo "Note: local HEAD is $local_head_sha; candidate source is pinned from origin/$MOBILE_RELEASE_SOURCE_BRANCH $source_sha." >&2
    fi
    printf 'Published %s at pinned origin/%s commit %s through buzz-release-bot. Use this exact tag in Release Mobile.\n' \
      "$tag" "$MOBILE_RELEASE_SOURCE_BRANCH" "$source_sha"
    ;;

  *) usage ;;
esac
