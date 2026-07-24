#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo_root/scripts/mobile-release.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
remote="$tmp/remote.git"
work="$tmp/work"
operator="$tmp/operator"
bin="$tmp/bin"
canonical_origin="git@github.com:block/buzz.git"
mkdir -p "$bin"

cat > "$bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}:${2:-}" in
  --version:*) printf 'gh version %s (test)\n' "${GH_VERSION:-2.94.0}" ;;
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
  workflow:run)
    if [[ "${GH_WORKFLOW_DISPATCH_FAIL:-}" == "1" ]]; then
      printf '%s\n' "${GH_WORKFLOW_DISPATCH_ERROR:-dispatch failed}" >&2
      exit 1
    fi
    if [[ "${GH_WORKFLOW_WRONG_URL:-}" == "1" ]]; then
      printf '%s\n' 'https://github.com/attacker/buzz/actions/runs/999'
      exit 0
    fi
    if [[ "${GH_WORKFLOW_EXTRA_URL:-}" == "1" ]]; then
      printf '%s\n' 'https://github.com/block/buzz/actions/runs/998'
    fi
    version=""
    number=""
    sha=""
    while [[ "$#" -gt 0 ]]; do
      case "$1" in
        version=*) version="${1#version=}" ;;
        candidate_number=*) number="${1#candidate_number=}" ;;
        target_sha=*) sha="${1#target_sha=}" ;;
      esac
      shift
    done
    [[ -n "$version" && -n "$number" && -n "$sha" ]]
    printf '%s\t%s\t%s\n' "$version" "$number" "$sha" >> "$GH_WORKFLOW_CAPTURE"
    if [[ "${GH_WORKFLOW_NO_URL:-}" == "1" ]]; then
      exit 0
    fi
    printf 'https://github.com/block/buzz/actions/runs/%s\n' "$number"
    ;;
  run:watch)
    [[ "${GH_WORKFLOW_FAIL:-}" != "1" ]] || exit 1
    number="$3"
    IFS=$'\t' read -r version expected sha < <(tail -n 1 "$GH_WORKFLOW_CAPTURE")
    [[ "$number" == "$expected" ]]
    if [[ "${GH_MAIN_MOVE_DURING_WORKFLOW:-}" == "1" ]]; then
      printf '%s\n' moved >> "$GH_WORKTREE/file"
      git -C "$GH_WORKTREE" commit -qam moved-during-publication
      git -C "$GH_WORKTREE" push -q origin main
    fi
    if [[ "${GH_TAG_VERIFY_LIGHTWEIGHT:-}" == "1" ]]; then
      git -C "$GH_WORKTREE" -c tag.gpgSign=false tag \
        "mobile-v${version}-rc.${expected}" "$sha"
    else
      git -C "$GH_WORKTREE" -c tag.gpgSign=false tag -a \
        -m "Buzz Mobile $version release candidate $expected" \
        "mobile-v${version}-rc.${expected}" "$sha"
    fi
    git -C "$GH_WORKTREE" -c core.hooksPath=/dev/null push -q \
      origin "refs/tags/mobile-v${version}-rc.${expected}"
    ;;
  *) exit 2 ;;
esac
GH
chmod +x "$bin/gh"

export PATH="$bin:$PATH"
export GH_WORKFLOW_CAPTURE="$tmp/workflow-dispatches"
export GH_WORKTREE="$work"

run_release() {
  local repo="$1"
  shift
  (
    cd "$repo"
    git config "url.file://$remote.insteadOf" "$canonical_origin"
    git config protocol.file.allow always
    "$script" "$@"
  )
}

fail() {
  echo "$*" >&2
  exit 1
}

assert_no_removed_mobile_release_behavior() {
  local status

  grep -Eq 'gh[[:space:]]+release|mobile-release/|finalize' "$@" && \
    fail "removed branch/finalization/GitHub Release behavior remains"
  status="$?"
  [[ "$status" -eq 1 ]] || \
    fail "could not scan for removed branch/finalization/GitHub Release behavior"
}

git init -q --bare "$remote"
git init -q "$work"
git -C "$work" config user.name test
git -C "$work" config user.email test@example.com
git -C "$work" remote add origin "$canonical_origin"
git -C "$work" config "url.file://$remote.insteadOf" "$canonical_origin"
git -C "$work" config protocol.file.allow always
echo first > "$work/file"
git -C "$work" add file
git -C "$work" commit -qm first
git -C "$work" branch -M main
git --git-dir="$remote" symbolic-ref HEAD refs/heads/main
git -C "$work" push -q -u origin main

# Candidate publication must work from a stale operator clone, warn about the
# stale checkout, and target the exact current remote main commit.
git -c "url.file://$remote.insteadOf=$canonical_origin" \
  -c protocol.file.allow=always clone -q "$canonical_origin" "$operator"
git -C "$operator" config user.name test
git -C "$operator" config user.email test@example.com
echo remote-only >> "$work/file"
git -C "$work" commit -qam remote-only
git -C "$work" push -q origin main
remote_main_sha="$(git --git-dir="$remote" rev-parse refs/heads/main)"
if git -C "$operator" cat-file -e "$remote_main_sha^{commit}" 2>/dev/null; then
  fail "stale-clone fixture already contains the remote-only commit"
fi
run_release "$operator" candidate 1.2.3 > "$tmp/stale-output" 2> "$tmp/stale-error"
grep -Fq "Note: local HEAD is $(git -C "$operator" rev-parse HEAD); candidate source is current origin/main $remote_main_sha." \
  "$tmp/stale-error"
cat "$tmp/stale-output"
[[ "$(git --git-dir="$remote" rev-parse 'refs/tags/mobile-v1.2.3-rc.1^{commit}')" == \
   "$remote_main_sha" ]]
[[ "$(git --git-dir="$remote" cat-file -t refs/tags/mobile-v1.2.3-rc.1)" == tag ]]
grep -Fq $'1.2.3\t1\t' "$GH_WORKFLOW_CAPTURE"

# Existing remote identities remain unchanged. Later candidates sequence
# monotonically and target the then-current remote main commit.
rc1_tag_oid="$(git --git-dir="$remote" rev-parse refs/tags/mobile-v1.2.3-rc.1)"
echo newer >> "$work/file"
git -C "$work" commit -qam newer
git -C "$work" push -q origin main
new_main_sha="$(git --git-dir="$remote" rev-parse refs/heads/main)"
run_release "$operator" candidate 1.2.3
[[ "$(git --git-dir="$remote" rev-parse refs/tags/mobile-v1.2.3-rc.1)" == "$rc1_tag_oid" ]]
[[ "$(git --git-dir="$remote" rev-parse 'refs/tags/mobile-v1.2.3-rc.2^{commit}')" == \
   "$new_main_sha" ]]
[[ "$(git --git-dir="$remote" cat-file -t refs/tags/mobile-v1.2.3-rc.2)" == tag ]]
grep -Fq $'1.2.3\t2\t' "$GH_WORKFLOW_CAPTURE"

# Sequence from the highest exact remote RC even if there are gaps, and ignore
# malformed or other-version tags.
git -C "$work" -c tag.gpgSign=false tag -a -m gap mobile-v1.2.3-rc.7 "$new_main_sha"
git -C "$work" -c tag.gpgSign=false tag -a -m malformed mobile-v1.2.3-rc.08 "$new_main_sha"
git -C "$work" -c tag.gpgSign=false tag -a -m other mobile-v1.2.4-rc.99 "$new_main_sha"
git -C "$work" push -q origin \
  refs/tags/mobile-v1.2.3-rc.7 refs/tags/mobile-v1.2.3-rc.08 \
  refs/tags/mobile-v1.2.4-rc.99
run_release "$operator" candidate 1.2.3
[[ "$(git --git-dir="$remote" rev-parse 'refs/tags/mobile-v1.2.3-rc.8^{commit}')" == \
   "$new_main_sha" ]]

# Failed or unattributable App-backed publication fails closed without creating
# the expected candidate tag.
if GH_WORKFLOW_DISPATCH_FAIL=1 run_release "$operator" candidate 9.9.7 >/dev/null 2>&1; then
  fail "candidate succeeded despite a rejected workflow dispatch"
fi
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v9.9.7-rc.1; then
  fail "rejected workflow dispatch created a candidate tag"
fi
if GH_WORKFLOW_DISPATCH_FAIL=1 \
    GH_WORKFLOW_DISPATCH_ERROR="does not have 'workflow_dispatch' trigger" \
    run_release "$operator" candidate 9.9.6 > "$tmp/missing-workflow-output" 2>&1; then
  fail "candidate succeeded without the publication workflow on main"
fi
grep -Fq 'merge the release-process change before publishing a candidate' \
  "$tmp/missing-workflow-output"
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v9.9.6-rc.1; then
  fail "missing publication workflow created a candidate tag"
fi
if GH_WORKFLOW_FAIL=1 run_release "$operator" candidate 9.9.9 >/dev/null 2>&1; then
  fail "candidate succeeded despite a failed App-backed workflow"
fi
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v9.9.9-rc.1; then
  fail "failed App-backed workflow created a candidate tag"
fi
if GH_WORKFLOW_NO_URL=1 run_release "$operator" candidate 9.9.8 >/dev/null 2>&1; then
  fail "candidate succeeded without a workflow run URL"
fi
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v9.9.8-rc.1; then
  fail "URL-less dispatch created a candidate tag"
fi
if GH_WORKFLOW_WRONG_URL=1 run_release "$operator" candidate 9.9.5 >/dev/null 2>&1; then
  fail "candidate accepted a workflow run URL from another repository"
fi
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v9.9.5-rc.1; then
  fail "wrong-repository workflow URL created a candidate tag"
fi
if GH_WORKFLOW_EXTRA_URL=1 run_release "$operator" candidate 9.9.4 >/dev/null 2>&1; then
  fail "candidate accepted multiple workflow run URLs"
fi
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v9.9.4-rc.1; then
  fail "ambiguous workflow URLs created a candidate tag"
fi
if GH_TAG_VERIFY_LIGHTWEIGHT=1 run_release "$operator" candidate 9.9.2 >/dev/null 2>&1; then
  fail "candidate accepted a lightweight published tag"
fi
[[ "$(git --git-dir="$remote" cat-file -t refs/tags/mobile-v9.9.2-rc.1)" == commit ]]

# The publisher and operator both reject a main-tip race. The immutable tag may
# already exist at the prior tip, but the operator must not report it as a
# current-main candidate.
pre_race_main_sha="$(git --git-dir="$remote" rev-parse refs/heads/main)"
if GH_MAIN_MOVE_DURING_WORKFLOW=1 run_release "$operator" candidate 9.9.3 > "$tmp/main-race-output" 2>&1; then
  fail "candidate succeeded after main moved during publication"
fi
post_race_main_sha="$(git --git-dir="$remote" rev-parse refs/heads/main)"
[[ "$pre_race_main_sha" != "$post_race_main_sha" ]]
[[ "$(git --git-dir="$remote" rev-parse 'refs/tags/mobile-v9.9.3-rc.1^{commit}')" == \
   "$pre_race_main_sha" ]]
grep -Fq "origin/main moved from requested commit $pre_race_main_sha to $post_race_main_sha during publication" \
  "$tmp/main-race-output"

# Publishing through a fork is rejected and unsupported gh versions fail before
# dispatching any candidate publication.
fork_operator="$tmp/fork-operator"
git clone -q "$remote" "$fork_operator"
git -C "$fork_operator" config user.name test
git -C "$fork_operator" config user.email test@example.com
if (cd "$fork_operator" && "$script" candidate 2.0.0 >/dev/null 2>&1); then
  fail "noncanonical origin was accepted"
fi
before_dispatches="$(wc -l < "$GH_WORKFLOW_CAPTURE")"
if GH_VERSION=2.86.0 run_release "$operator" candidate 2.0.0 >/dev/null 2>&1; then
  fail "gh older than 2.87.0 was accepted"
fi
if GH_VERSION=2.9.0 run_release "$operator" candidate 2.0.0 >/dev/null 2>&1; then
  fail "numeric gh version comparison accepted 2.9.0 as at least 2.87.0"
fi
[[ "$(wc -l < "$GH_WORKFLOW_CAPTURE")" == "$before_dispatches" ]]

# Dirty trees and invalid marketing versions fail before publication.
echo dirty > "$operator/untracked"
if run_release "$operator" candidate 2.0.0 >/dev/null 2>&1; then
  fail "dirty operator tree was accepted"
fi
rm "$operator/untracked"
if run_release "$operator" candidate 1.2 >/dev/null 2>&1; then
  fail "invalid marketing version was accepted"
fi
if run_release "$operator" candidate 01.2.3 >/dev/null 2>&1; then
  fail "marketing version with a leading zero was accepted"
fi

# Mobile no longer has release branches, finalization, a stable alias, or a
# GitHub Release call. Publication remains App-backed because the strict tag
# ruleset denies direct human creation.
if run_release "$operator" start 2.0.0 >/dev/null 2>&1; then
  fail "removed start command was accepted"
fi
if run_release "$operator" finalize 1.2.3-rc.2 >/dev/null 2>&1; then
  fail "removed finalize command was accepted"
fi
if git --git-dir="$remote" for-each-ref --format='%(refname)' refs/heads/mobile-release/ | grep -q .; then
  fail "mobile release branch was created"
fi
if git --git-dir="$remote" show-ref --verify --quiet refs/tags/mobile-v1.2.3; then
  fail "stable mobile tag alias was created"
fi
assert_no_removed_mobile_release_behavior \
  "$repo_root/scripts/mobile-release.sh" \
  "$repo_root/scripts/publish-mobile-release-candidate.sh" \
  "$repo_root/.github/workflows/mobile-release-candidate.yml"
# Prove the negative contract itself is discriminating rather than merely
# accepting a missing or broken search tool as "not found."
printf '%s\n' 'gh release create forbidden' > "$tmp/forbidden-mobile-release-behavior"
if (assert_no_removed_mobile_release_behavior "$tmp/forbidden-mobile-release-behavior") \
    >/dev/null 2>&1; then
  fail "removed-behavior assertion did not reject a forbidden GitHub Release call"
fi
grep -Fq 'version: 0.0.0+1' "$repo_root/mobile/pubspec.yaml"
if grep -qE 'release-mobile|bump-mobile-version|get-current-mobile-version' "$repo_root/Justfile"; then
  fail "metadata-only mobile release recipe remains in Justfile"
fi

echo "mobile release contract passed"
