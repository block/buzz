#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage:
  scripts/mobile-release.sh candidate X.Y.Z [--source-branch mobile-vX.Y] [--commit FULL_SHA]

candidate  Publish the next immutable mobile-vX.Y.Z-rc.N candidate tag at the
           selected commit in source history (defaults to the current source tip).
           You push the annotated tag yourself, so you must be a bypass actor
           of the mobile tag ruleset (currently the mobile release managers).
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
  local ref="refs/heads/$MOBILE_RELEASE_SOURCE_BRANCH" line oid commit
  # Take a single snapshot of the branch tip from origin itself. A later branch
  # advance does not invalidate it. The shared FETCH_HEAD is never read: a
  # concurrent fetch in this checkout could replace it with another branch.
  line="$(git ls-remote --refs origin "$ref")" || return 1
  [[ -n "$line" && "$line" != *$'\n'* ]] || return 1
  oid="${line%%$'\t'*}"
  [[ "$oid" =~ ^[0-9a-f]{40}$ ]] || return 1
  git fetch -q --no-tags origin "$oid" || return 1
  commit="$(git rev-parse --verify "$oid^{commit}")" || return 1
  [[ "$commit" == "$oid" ]] || \
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
    # Candidate tags are pushed by the operator, who must be a bypass actor of
    # the mobile tag ruleset (the mobile release managers). No GitHub App or
    # workflow is involved. Check before creating anything locally.
    require_mobile_release_tag_ruleset || exit 1
    # A pinned --commit can be a verified ancestor outside a shallow clone's
    # history. Fetch that exact commit so the local tag can point at it.
    if ! git cat-file -e "$source_sha^{commit}" 2>/dev/null; then
      git fetch -q --no-tags origin "$source_sha" || fail "could not fetch source commit $source_sha"
    fi
    # Annotated, never forced; the operator's own tag signing config applies.
    git tag -a -m "Buzz Mobile $version release candidate $next" "$tag" "$source_sha" || \
      fail "could not create local tag $tag"
    # --no-verify: the release must not depend on development pre-push hooks.
    # --no-follow-tags and an explicit refspec: push exactly this one ref, even
    # when push.followTags would add other local annotated tags.
    if ! git push --no-verify --no-follow-tags origin "refs/tags/$tag:refs/tags/$tag"; then
      # Only delete the local tag when origin certainly lacks the candidate.
      if ! remote_line="$(git ls-remote --refs origin "refs/tags/$tag")"; then
        fail "could not push $tag, and could not check whether origin has it. If GitHub shows $tag, use it and don't rerun; otherwise run 'git tag -d $tag' before retrying"
      fi
      if [[ -z "$remote_line" ]]; then
        git tag -d "$tag" >/dev/null
        fail "could not push $tag; you must be a bypass actor of mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID (currently the mobile release managers). The local tag was removed so a retry starts clean"
      fi
      # origin has the tag, but origin may be rewritten to a fork; the checks
      # below decide whether canonical block/buzz has exactly this candidate.
      echo "Note: push of $tag reported failure, but origin has $tag; verifying it on canonical block/buzz." >&2
    fi

    # git URL rewriting (insteadOf) can send even a canonical-looking origin
    # to a fork, so confirm with GitHub itself that block/buzz now has this
    # exact annotated tag object. An unavailable API and a mismatch both stop
    # here without deleting the local tag.
    local_tag_oid="$(git rev-parse --verify "refs/tags/$tag")" || fail "local tag $tag disappeared"
    if ! canonical_tag_oid="$(gh api "repos/block/buzz/git/ref/tags/$tag" --jq .object.sha)" || \
        [[ "$canonical_tag_oid" != "$local_tag_oid" ]]; then
      # Numbering reads only remote tags, so keeping the local tag doesn't
      # change the next candidate number. It records what was attempted, and
      # after a fork rewrite is fixed it must be deleted before rerunning.
      fail "could not confirm $tag on canonical block/buzz. If GitHub shows $tag, use it and don't rerun. If it doesn't, fix any git url.*.insteadOf rewrite for origin and run 'git tag -d $tag' before retrying"
    fi
    published_sha="$(remote_tag_commit_sha "refs/tags/$tag")" || \
      fail "push completed without exact annotated candidate tag $tag on origin"
    [[ "$published_sha" == "$source_sha" ]] || \
      fail "$tag resolved to $published_sha instead of requested commit $source_sha"
    if [[ "$local_head_sha" != "$source_sha" ]]; then
      echo "Note: local HEAD is $local_head_sha; candidate source is pinned from origin/$MOBILE_RELEASE_SOURCE_BRANCH $source_sha." >&2
    fi
    printf 'Published %s at pinned origin/%s commit %s. Use this exact tag in Release Mobile.\n' \
      "$tag" "$MOBILE_RELEASE_SOURCE_BRANCH" "$source_sha"
    ;;

  *) usage ;;
esac
