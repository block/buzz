#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage:
  scripts/mobile-release.sh candidate X.Y.Z

candidate  Publish the next immutable mobile-vX.Y.Z-rc.N candidate tag at the
           exact current commit of block/buzz's remote main branch.
USAGE
  exit 2
}

fail() {
  echo "Error: $*" >&2
  exit 1
}

# shellcheck source=scripts/release-rulesets.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-rulesets.sh"

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

remote_main_commit_sha() {
  local ref="refs/heads/main" line advertised_oid fetched_oid commit
  line="$(git ls-remote --refs origin "$ref")" || return 1
  [[ -n "$line" && "$line" != *$'\n'* ]] || return 1
  advertised_oid="${line%%$'\t'*}"
  git fetch -q --no-tags origin "$ref"
  fetched_oid="$(git rev-parse --verify FETCH_HEAD)"
  [[ "$fetched_oid" == "$advertised_oid" ]] || \
    fail "origin/main moved while it was being resolved"
  commit="$(git rev-parse --verify 'FETCH_HEAD^{commit}')" || return 1
  [[ "$commit" == "$advertised_oid" ]] || \
    fail "origin/main did not resolve directly to a commit"
  printf '%s' "$commit"
}

command="${1:-}"
case "$command" in
  candidate)
    [[ "$#" -eq 2 ]] || usage
    version="$2"
    require_clean_semver "$version"
    require_clean_tree
    require_canonical_repository || exit 1
    require_gh_minimum_version

    local_head_sha="$(git rev-parse --verify 'HEAD^{commit}')" || fail "HEAD is not a commit"
    main_sha="$(remote_main_commit_sha)" || fail "origin/main does not exist"
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
      -f "target_sha=$main_sha" 2>&1)"; then
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

    current_main_sha="$(remote_main_commit_sha)" || fail "origin/main does not exist after publication"
    [[ "$current_main_sha" == "$main_sha" ]] || \
      fail "origin/main moved from requested commit $main_sha to $current_main_sha during publication"
    published_sha="$(remote_tag_commit_sha "refs/tags/$tag")" || \
      fail "publication completed without exact annotated candidate tag $tag"
    [[ "$published_sha" == "$main_sha" ]] || \
      fail "$tag resolved to $published_sha instead of requested commit $main_sha"
    if [[ "$local_head_sha" != "$main_sha" ]]; then
      echo "Note: local HEAD is $local_head_sha; candidate source is current origin/main $main_sha." >&2
    fi
    printf 'Published %s at origin/main commit %s through buzz-release-bot. Use this exact tag in Release Mobile.\n' \
      "$tag" "$main_sha"
    ;;

  *) usage ;;
esac
