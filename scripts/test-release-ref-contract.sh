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
auto_tag="$repo_root/.github/workflows/auto-tag-on-release-pr-merge.yml"
grep -q 'actions/create-github-app-token@' "$auto_tag"
grep -q 'client-id:.*vars\.BUZZ_RELEASE_TAGGER_CLIENT_ID' "$auto_tag"
grep -q 'private-key:.*secrets\.BUZZ_RELEASE_TAGGER_PRIVATE_KEY' "$auto_tag"
grep -q 'permission-contents: write' "$auto_tag"
grep -q 'GH_TOKEN:.*steps\.release-tagger\.outputs\.token' "$auto_tag"
grep -Fq 'git/refs' "$auto_tag"
grep -Fq 'TAG_PREFIX="desktop-v"' "$auto_tag"
grep -Fq 'target_sha=${{ github.event.pull_request.merge_commit_sha }}' "$auto_tag"
grep -Fq 'scripts/verify-desktop-release-merge.sh' "$auto_tag"
"$repo_root/scripts/test-desktop-release-authorization.sh"
if rg -q 'rule-suites|desktop-release-bypass-authorized|MERGED_BY' \
  "$repo_root/scripts/verify-desktop-release-merge.sh" \
  "$repo_root/scripts/verify-desktop-release-authorization.sh" \
  "$auto_tag"; then
  echo "desktop auto-tag still depends on unavailable rule-suite authorization" >&2
  exit 1
fi

review_filter="$repo_root/scripts/review-decision-approved.jq"
for fixture in \
  '{"reviewDecision":"CHANGES_REQUESTED"}' \
  '{"reviewDecision":"REVIEW_REQUIRED"}' \
  '{"reviewDecision":null}' \
  '{}'; do
  if jq -e -f "$review_filter" <<<"$fixture" >/dev/null; then
    echo "review-decision filter accepted non-approved fixture: $fixture" >&2
    exit 1
  fi
done
jq -e -f "$review_filter" >/dev/null <<'JSON' || {
{"reviewDecision":"APPROVED"}
JSON
  echo "review-decision filter rejected approved GraphQL response" >&2
  exit 1
}
required_check_filter="$repo_root/scripts/required-check-succeeded.jq"
check_fixture() {
  local expected="$1" conclusion="$2" status="${3:-completed}"
  local payload
  payload=$(jq -n --arg status "$status" --arg conclusion "$conclusion" '{check_runs: [{name: "Web", status: $status, conclusion: $conclusion, started_at: "2026-01-01T00:00:00Z"}]}')
  if jq -e --arg name Web -f "$required_check_filter" <<<"[$payload]" >/dev/null; then
    actual=pass
  else
    actual=fail
  fi
  [[ "$actual" == "$expected" ]] || {
    echo "required-check filter: expected $conclusion/$status to $expected" >&2
    exit 1
  }
}
check_fixture pass success
check_fixture pass skipped
check_fixture pass neutral
check_fixture fail failure
check_fixture fail success in_progress
# A newer failure must not be hidden by an older successful run of the same check.
jq -e --arg name Web -f "$required_check_filter" >/dev/null <<'JSON' && {
[{"check_runs":[
  {"name":"Web","status":"completed","conclusion":"success","started_at":"2026-01-01T00:00:00Z"},
  {"name":"Web","status":"completed","conclusion":"failure","started_at":"2026-01-02T00:00:00Z"}
]}]
JSON
  echo "required-check filter accepted a stale pass over a newer failure" >&2
  exit 1
}
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
