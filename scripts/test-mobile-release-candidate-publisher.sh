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
  api:repos/block/buzz/git/ref/heads/mobile-v1.2) printf '%s\n' "$GH_STABLE_SHA" ;;
  api:repos/block/buzz/git/ref/heads/*) printf '%s\n' "$GH_MISMATCH_SHA" ;;
  api:repos/block/buzz/commits/*) printf '%s\n' "${2##*/}" ;;
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
export GH_STABLE_SHA=4444444444444444444444444444444444444444
export GH_MISMATCH_SHA=5555555555555555555555555555555555555555
export GH_TAG_OBJECT_SHA=2222222222222222222222222222222222222222

"$publisher" 1.2.3 1 "$GH_TARGET_SHA" main
grep -Fq -- '-f tag=mobile-v1.2.3-rc.1' "$GH_CALLS"
grep -Fq -- '-f message=Buzz Mobile 1.2.3 release candidate 1' "$GH_CALLS"
grep -Fq -- "-f object=$GH_TARGET_SHA" "$GH_CALLS"
grep -Fq -- '-f type=commit' "$GH_CALLS"
grep -Fq -- '-f ref=refs/tags/mobile-v1.2.3-rc.1' "$GH_CALLS"
grep -Fq -- "-f sha=$GH_TAG_OBJECT_SHA" "$GH_CALLS"

if GH_CURRENT_USER_CAN_BYPASS=never "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted an App token without an always bypass" >&2
  exit 1
fi
if GH_CURRENT_USER_CAN_BYPASS='' "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted a ruleset response without an effective bypass" >&2
  exit 1
fi
if GH_TAG_RULESET_STATE=disabled "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted disabled tag protection" >&2
  exit 1
fi
if GH_TAG_RULE_TYPES=creation "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted incomplete tag protection" >&2
  exit 1
fi
if GH_TAG_INCLUDES=refs/tags/v\* "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted a tag ruleset that excludes mobile candidates" >&2
  exit 1
fi
if GH_TAG_EXCLUDES=refs/tags/mobile-v0.0.0 "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted tag ruleset exclusions" >&2
  exit 1
fi
if GH_TARGET_SHA=3333333333333333333333333333333333333333 \
    "$publisher" 1.2.3 1 1111111111111111111111111111111111111111 main >/dev/null 2>&1; then
  echo "publisher accepted a moved main branch" >&2
  exit 1
fi
if GH_EXISTING_REFS=$'refs/tags/mobile-v1.2.3-rc.1\n' \
    "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted a stale candidate number" >&2
  exit 1
fi
if GH_PUBLISHED_REF_TYPE=commit "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted a lightweight published tag" >&2
  exit 1
fi
if GH_PUBLISHED_REF_SHA=3333333333333333333333333333333333333333 \
    "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted the wrong annotated tag object" >&2
  exit 1
fi
if GH_ANNOTATED_TARGET_TYPE=tag "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted a nested annotated tag" >&2
  exit 1
fi
if GH_ANNOTATED_TARGET_SHA=3333333333333333333333333333333333333333 \
    "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted an annotated tag on the wrong commit" >&2
  exit 1
fi
if "$publisher" 01.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted a marketing version with a leading zero" >&2
  exit 1
fi
if "$publisher" 1.2.3 01 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted a candidate number with a leading zero" >&2
  exit 1
fi
if GH_EXISTING_REFS=$'refs/tags/mobile-v1.2.3-rc.1\nrefs/tags/mobile-v1.2.3-rc.7\nrefs/tags/mobile-v1.2.3-rc.08\nrefs/tags/mobile-v1.2.4-rc.99\n' \
    "$publisher" 1.2.3 8 "$GH_TARGET_SHA" main >/dev/null; then
  :
else
  echo "publisher did not sequence from the highest exact candidate" >&2
  exit 1
fi
if GITHUB_REPOSITORY=attacker/buzz "$publisher" 1.2.3 1 "$GH_TARGET_SHA" main >/dev/null 2>&1; then
  echo "publisher accepted the wrong repository" >&2
  exit 1
fi

GH_ANNOTATED_TARGET_SHA="$GH_STABLE_SHA" "$publisher" 1.2.3 1 "$GH_STABLE_SHA" mobile-v1.2
for source in mobile-v1.3 mobile-v2.2 mobile-v01.2 arbitrary ''; do
  if GH_ANNOTATED_TARGET_SHA="$GH_MISMATCH_SHA" "$publisher" 1.2.3 1 "$GH_MISMATCH_SHA" "$source" >/dev/null 2>&1; then
    echo "publisher accepted mismatched source '$source'" >&2
    exit 1
  fi
done
# Exercise the checked-in workflow command with its actual source input binding.
python3 - "$repo_root/.github/workflows/mobile-release-candidate.yml" "$tmp" <<'PYWORKFLOW'
import os
from pathlib import Path
import re
import subprocess
import sys
workflow = Path(sys.argv[1]).read_text()
step = workflow.split("      - name: Publish annotated candidate tag\n", 1)[1]
assert re.search(r"^          SOURCE_BRANCH: \$\{\{ inputs\.source_branch \}\}$", step, re.M), "source input is not bound to SOURCE_BRANCH"
command = re.search(r"^        run: (.+)$", step, re.M).group(1)
root = Path(sys.argv[2]) / "workflow"
(root / "scripts").mkdir(parents=True)
stub = root / "scripts/publish-mobile-release-candidate.sh"
stub.write_text('#!/usr/bin/env bash\nprintf "%s\\n" "$@"\n')
stub.chmod(0o755)
env = dict(os.environ, MOBILE_VERSION="1.2.3", CANDIDATE_NUMBER="1", TARGET_SHA="4" * 40, SOURCE_BRANCH="mobile-v1.2")
actual = subprocess.check_output(["bash", "-eu", "-c", command], cwd=root, env=env, text=True).splitlines()
assert actual == ["1.2.3", "1", "4" * 40, "mobile-v1.2"], actual
PYWORKFLOW

echo "mobile release candidate publisher contract passed"
