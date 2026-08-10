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
git -C "$tmp" tag -m "desktop release" desktop-v1.2.3

(
  cd "$tmp"
  GITHUB_REF=refs/tags/desktop-v1.2.3 "$verify" desktop-v 1.2.3
)

if (
  cd "$tmp"
  GITHUB_REF=refs/heads/main "$verify" desktop-v 1.2.3
); then
  echo "branch-backed desktop release was accepted" >&2
  exit 1
fi

echo second >>"$tmp/file"
git -C "$tmp" commit -qam second
if (
  cd "$tmp"
  GITHUB_REF=refs/tags/desktop-v1.2.3 "$verify" desktop-v 1.2.3
); then
  echo "release accepted HEAD after the tag commit" >&2
  exit 1
fi

git -C "$tmp" tag -m "relay release" relay-v2.0.0
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
"$repo_root/scripts/test-desktop-release-cache-key.sh"
"$repo_root/scripts/test-desktop-release-cache-workflow.sh"
auto_tag="$repo_root/.github/workflows/auto-tag-on-release-pr-merge.yml"
grep -q 'actions/create-github-app-token@' "$auto_tag"
grep -q 'client-id:.*vars\.BUZZ_RELEASE_TAGGER_CLIENT_ID' "$auto_tag"
grep -q 'private-key:.*secrets\.BUZZ_RELEASE_TAGGER_PRIVATE_KEY' "$auto_tag"
grep -q 'permission-contents: write' "$auto_tag"
grep -q 'GH_TOKEN:.*steps\.release-tagger\.outputs\.token' "$auto_tag"
grep -Fq 'git/refs' "$auto_tag"
grep -Fq 'TAG_PREFIX="desktop-v"' "$auto_tag"
grep -Fq 'target_sha=${{ github.event.pull_request.head.sha }}' "$auto_tag"
grep -Fq 'scripts/verify-desktop-release-merge.sh' "$auto_tag"
candidate_workflow="$repo_root/.github/workflows/desktop-release-candidate.yml"
grep -Eq '^  pull-requests: read$' "$candidate_workflow" || {
  echo "desktop candidate token cannot read pull requests for prior-release lookup" >&2
  exit 1
}
grep -Fq 'GH_TOKEN: ${{ github.token }}' "$candidate_workflow" || {
  echo "desktop candidate validation has no GitHub token for prior-release lookup" >&2
  exit 1
}
grep -Fq 'reviewed candidate' "$repo_root/scripts/prepare-desktop-release.sh"
if grep -Fq 'current `main`' "$repo_root/scripts/prepare-desktop-release.sh"; then
  echo "desktop release PR body contains executable command substitution" >&2
  exit 1
fi
required_check_filter="$repo_root/scripts/required-check-succeeded.jq"
check_fixture() {
  local expected="$1" conclusion="$2" app="${3:-15368}" completed="${4:-2026-01-01T00:00:00Z}"
  local payload actual
  # Production-shaped REST check run: notably, there is no created_at field.
  payload=$(jq -n --arg conclusion "$conclusion" --argjson app "$app" --arg completed "$completed" \
    '{check_runs: [{id: 100, check_suite: {id: 10}, name: "Web", app: {id: $app}, status: "completed", conclusion: $conclusion, started_at: "2026-01-01T00:00:00Z", completed_at: $completed}]}')
  if jq -e --arg name Web --argjson integration_id 15368 \
    --arg merged_at 2026-01-02T00:00:00Z \
    -f "$required_check_filter" <<<"[$payload]" >/dev/null; then actual=pass; else actual=fail; fi
  [[ "$actual" == "$expected" ]] || { echo "required-check fixture expected $expected, got $actual" >&2; exit 1; }
}
check_fixture pass success
check_fixture pass skipped
check_fixture pass neutral
check_fixture fail failure
check_fixture fail success 999
check_fixture fail success 15368 2026-01-03T00:00:00Z

# filter=latest may still return multiple same-name runs from distinct workflows.
# Highest immutable run ID is authoritative and must not reveal stale green.
jq -e --arg name Web --argjson integration_id 15368 --arg merged_at 2026-01-02T00:00:00Z \
  -f "$required_check_filter" >/dev/null <<'JSON' && {
[{"check_runs":[
  {"id":100,"check_suite":{"id":10},"name":"Web","app":{"id":15368},"status":"completed","conclusion":"success","started_at":"2026-01-01T00:00:00Z","completed_at":"2026-01-01T01:00:00Z"},
  {"id":101,"check_suite":{"id":11},"name":"Web","app":{"id":15368},"status":"in_progress","conclusion":null,"started_at":"2026-01-01T23:59:00Z","completed_at":null}
]}]
JSON
  echo "required-check filter hid the highest-ID pending attempt" >&2; exit 1;
}
# A post-merge rerun is indistinguishable from other latest attempts and fails closed.
jq -e --arg name Web --argjson integration_id 15368 --arg merged_at 2026-01-02T00:00:00Z \
  -f "$required_check_filter" >/dev/null <<'JSON' && {
[{"check_runs":[
  {"id":100,"check_suite":{"id":10},"name":"Web","app":{"id":15368},"status":"completed","conclusion":"success","started_at":"2026-01-01T00:00:00Z","completed_at":"2026-01-01T01:00:00Z"},
  {"id":101,"check_suite":{"id":10},"name":"Web","app":{"id":15368},"status":"completed","conclusion":"failure","started_at":"2026-01-02T00:01:00Z","completed_at":"2026-01-02T00:10:00Z"}
]}]
JSON
  echo "required-check filter accepted stale success after post-merge rerun" >&2; exit 1;
}
# DCO alone may complete just after merge, inside its explicit five-minute bound.
dco_fixture() {
  local expected="$1" completed="$2" actual
  if jq -e --arg name "DCO Check" --argjson integration_id 1455659 --arg merged_at 2026-01-02T00:00:00Z \
    -f "$required_check_filter" >/dev/null <<JSON
[{"check_runs":[{"id":200,"check_suite":{"id":20},"name":"DCO Check","app":{"id":1455659},"status":"completed","conclusion":"success","started_at":"$completed","completed_at":"$completed"}]}]
JSON
  then actual=pass; else actual=fail; fi
  [[ "$actual" == "$expected" ]] || { echo "DCO completion $completed expected $expected" >&2; exit 1; }
}
dco_fixture pass 2026-01-02T00:04:59Z
dco_fixture fail 2026-01-02T00:05:01Z

# The verifier must request production endpoint semantics and pin helpers before checkout.
verify_merge="$repo_root/scripts/verify-desktop-release-merge.sh"
grep -Fq 'check-runs?filter=latest&per_page=100' "$verify_merge"
grep -Fq 'git fetch origin main --no-tags' "$verify_merge"
grep -Fq 'git merge-base --is-ancestor "$candidate_parents" origin/main' "$verify_merge"
grep -Fq 'git show "$candidate_parents:scripts/desktop_release.py"' "$verify_merge"
grep -Fq 'git show "$candidate_parents:scripts/required-check-succeeded.jq"' "$verify_merge"
grep -Fq 'DESKTOP_RELEASE_ROOT="$PWD" python3 "$verifier_dir/desktop_release.py"' "$verify_merge"
grep -Fq -- '-f "$verifier_dir/required-check-succeeded.jq"' "$verify_merge"

release_workflow="$repo_root/.github/workflows/release.yml"
[[ "$(grep -c 'contents: write' "$release_workflow")" -eq 1 ]] || {
  echo "desktop release must have exactly one GitHub contents writer" >&2; exit 1;
}
grep -Fq "needs.release.result == 'success'" "$release_workflow"
grep -Fq "needs.release-macos-x64.result == 'success'" "$release_workflow"
grep -Fq "needs.release-linux.result == 'success'" "$release_workflow"
grep -Fq "needs.release-windows.result == 'success'" "$release_workflow"
grep -Fq "refs/tags/desktop-v{0}" "$release_workflow"
grep -Fq "if: \${{ !contains(needs.setup.outputs.version, '-') }}" "$release_workflow"
if grep -Fq "env.already_published != 'true' && !contains(needs.setup.outputs.version, '-')" "$release_workflow"; then
  echo "rolling updater retry is incorrectly gated by versioned publication state" >&2; exit 1
fi
grep -Fq 'group: desktop-release-${{ github.ref }}' "$release_workflow"
grep -Fq 'cancel-in-progress: false' "$release_workflow"
grep -Fq 'release artifact basename collision' "$release_workflow"
[[ "$(grep -c 'gh release upload' "$release_workflow")" -eq 2 ]] || {
  echo "only the final writer may upload versioned and rolling release assets" >&2; exit 1;
}
grep -Fq 'if: env.already_published' "$release_workflow"
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
