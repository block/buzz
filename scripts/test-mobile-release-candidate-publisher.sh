#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
publisher="$repo_root/scripts/publish-mobile-release-candidate.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

bin="$tmp/bin"
mkdir -p "$bin"
cat > "$bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail

record() {
  printf '%s\n' "$*" >> "$GH_CALLS"
}

case "${1:-}:${2:-}" in
  api:repos/block/buzz/rulesets/14378754)
    case "$*" in
      *'.enforcement'*) printf '%s\n' "${GH_TAG_RULESET_STATE:-active}" ;;
      *'.current_user_can_bypass'*) printf '%s\n' "${GH_CURRENT_USER_CAN_BYPASS-always}" ;;
      *'[.rules[].type]'*) printf '%s\n' "${GH_TAG_RULE_TYPES:-creation,deletion,non_fast_forward,update}" ;;
      *'.conditions.ref_name.include[]'*) printf '%s\n' "${GH_TAG_INCLUDES:-refs/tags/mobile-v*}" ;;
      *'.conditions.ref_name.exclude[]'*) printf '%s\n' "${GH_TAG_EXCLUDES:-}" ;;
      *) exit 2 ;;
    esac
    ;;
  api:repos/block/buzz/git/ref/heads/main) printf '%s\n' "$GH_TARGET_SHA" ;;
  api:repos/block/buzz/commits/*) printf '%s\n' "$GH_TARGET_SHA" ;;
  api:--paginate)
    [[ "$3" == "repos/block/buzz/git/matching-refs/tags/mobile-v1.2.3-rc." ]]
    printf '%s' "${GH_EXISTING_REFS:-}"
    ;;
  api:--method)
    endpoint="$4"
    case "$endpoint" in
      repos/block/buzz/git/tags)
        record "$*"
        printf '%s\n' "$GH_TAG_OBJECT_SHA"
        ;;
      repos/block/buzz/git/refs)
        record "$*"
        ;;
      *) exit 2 ;;
    esac
    ;;
  api:repos/block/buzz/git/ref/tags/mobile-v1.2.3-rc.*)
    if [[ "$*" == *'.object.type'* ]]; then
      printf '%s\n' "${GH_PUBLISHED_REF_TYPE:-tag}"
    else
      printf '%s\n' "${GH_PUBLISHED_REF_SHA:-$GH_TAG_OBJECT_SHA}"
    fi
    ;;
  api:repos/block/buzz/git/tags/*)
    if [[ "$*" == *'.object.type'* ]]; then
      printf '%s\n' "${GH_ANNOTATED_TARGET_TYPE:-commit}"
    else
      printf '%s\n' "${GH_ANNOTATED_TARGET_SHA:-$GH_TARGET_SHA}"
    fi
    ;;
  *)
    echo "unexpected gh call: $*" >&2
    exit 2
    ;;
esac
GH
chmod +x "$bin/gh"

export PATH="$bin:$PATH"
export GH_CALLS="$tmp/calls"
export GITHUB_REPOSITORY=block/buzz
export GH_TARGET_SHA=1111111111111111111111111111111111111111
export GH_TAG_OBJECT_SHA=2222222222222222222222222222222222222222

"$publisher" 1.2.3 1 "$GH_TARGET_SHA"
grep -Fq -- '-f tag=mobile-v1.2.3-rc.1' "$GH_CALLS"
grep -Fq -- '-f message=Buzz Mobile 1.2.3 release candidate 1' "$GH_CALLS"
grep -Fq -- "-f object=$GH_TARGET_SHA" "$GH_CALLS"
grep -Fq -- '-f type=commit' "$GH_CALLS"
grep -Fq -- '-f ref=refs/tags/mobile-v1.2.3-rc.1' "$GH_CALLS"
grep -Fq -- "-f sha=$GH_TAG_OBJECT_SHA" "$GH_CALLS"

if GH_CURRENT_USER_CAN_BYPASS=never "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted an App token without an always bypass" >&2
  exit 1
fi
if GH_CURRENT_USER_CAN_BYPASS='' "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted a ruleset response without an effective bypass" >&2
  exit 1
fi
if GH_TAG_RULESET_STATE=disabled "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted disabled tag protection" >&2
  exit 1
fi
if GH_TAG_RULE_TYPES=creation "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted incomplete tag protection" >&2
  exit 1
fi
if GH_TAG_INCLUDES=refs/tags/v\* "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted a tag ruleset that excludes mobile candidates" >&2
  exit 1
fi
if GH_TAG_EXCLUDES=refs/tags/mobile-v0.0.0 "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted tag ruleset exclusions" >&2
  exit 1
fi
if GH_TARGET_SHA=3333333333333333333333333333333333333333 \
    "$publisher" 1.2.3 1 1111111111111111111111111111111111111111 >/dev/null 2>&1; then
  echo "publisher accepted a moved main branch" >&2
  exit 1
fi
if GH_EXISTING_REFS=$'refs/tags/mobile-v1.2.3-rc.1\n' \
    "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted a stale candidate number" >&2
  exit 1
fi
if GH_PUBLISHED_REF_TYPE=commit "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted a lightweight published tag" >&2
  exit 1
fi
if GH_PUBLISHED_REF_SHA=3333333333333333333333333333333333333333 \
    "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted the wrong annotated tag object" >&2
  exit 1
fi
if GH_ANNOTATED_TARGET_TYPE=tag "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted a nested annotated tag" >&2
  exit 1
fi
if GH_ANNOTATED_TARGET_SHA=3333333333333333333333333333333333333333 \
    "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted an annotated tag on the wrong commit" >&2
  exit 1
fi
if "$publisher" 01.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted a marketing version with a leading zero" >&2
  exit 1
fi
if "$publisher" 1.2.3 01 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted a candidate number with a leading zero" >&2
  exit 1
fi
if GH_EXISTING_REFS=$'refs/tags/mobile-v1.2.3-rc.1\nrefs/tags/mobile-v1.2.3-rc.7\nrefs/tags/mobile-v1.2.3-rc.08\nrefs/tags/mobile-v1.2.4-rc.99\n' \
    "$publisher" 1.2.3 8 "$GH_TARGET_SHA" >/dev/null; then
  :
else
  echo "publisher did not sequence from the highest exact candidate" >&2
  exit 1
fi
if GITHUB_REPOSITORY=attacker/buzz "$publisher" 1.2.3 1 "$GH_TARGET_SHA" >/dev/null 2>&1; then
  echo "publisher accepted the wrong repository" >&2
  exit 1
fi

echo "mobile release candidate publisher contract passed"
