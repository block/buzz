#!/usr/bin/env bash
set -euo pipefail

version="${1:-}"
mode="${2:-publish}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || {
  echo "usage: $0 <semver> [publish|validate-only]" >&2
  exit 1
}

remote="${RELEASE_REMOTE:-origin}"
git fetch "$remote" refs/heads/main:refs/remotes/origin/main --no-tags
git fetch "$remote" '+refs/tags/v*:refs/tags/v*' '+refs/tags/desktop-v*:refs/tags/desktop-v*'
base_sha="$(git rev-parse refs/remotes/origin/main)"
branch="version-bump/$version"

remote_branch="refs/heads/$branch"
remote_oid=""
if remote_oid="$(git ls-remote "$remote" "$remote_branch" | awk '{print $1}')" && [[ -n "$remote_oid" ]]; then
  git fetch "$remote" "$remote_branch:refs/remotes/origin/$branch"
fi

git checkout -B "$branch" "$base_sha"
just bump-desktop-version "$version"
scripts/desktop_release.py generate "$version" --base "$base_sha" --repo block/buzz

git add \
  .release/desktop-candidate.json \
  CHANGELOG.md \
  desktop/package.json \
  desktop/src-tauri/tauri.conf.json \
  desktop/src-tauri/Cargo.toml \
  desktop/src-tauri/Cargo.lock \
  pnpm-lock.yaml

agent_name="${RELEASE_AUTOMATION_NAME:-${AGENT_NAME:-Release Automation}}"
agent_email="${RELEASE_AUTOMATION_EMAIL:-${AGENT_EMAIL:-release-automation@users.noreply.github.com}}"
msg="$(mktemp)"
trap 'rm -f "$msg"' EXIT
cat >"$msg" <<EOF
chore(release): release Buzz Desktop version $version

Co-authored-by: $agent_name <$agent_email>
EOF
git -c user.name='Wes' -c user.email='wesbillman@users.noreply.github.com' \
  commit -s -F "$msg"
scripts/desktop_release.py validate --candidate HEAD --version "$version" --repo block/buzz

candidate_sha="$(git rev-parse HEAD)"
previous_tag="$(python3 -c 'import json; print(json.load(open(".release/desktop-candidate.json"))["previous_tag"] or "initial")')"
printf 'base_sha=%s\ncandidate_sha=%s\nprevious_tag=%s\ntag=desktop-v%s\n' \
  "$base_sha" "$candidate_sha" "$previous_tag" "$version"

if [[ "$mode" == validate-only ]]; then
  exit 0
fi
[[ "$mode" == publish ]] || { echo "unknown mode: $mode" >&2; exit 1; }
if [[ -n "$remote_oid" ]]; then
  git push --force-with-lease="$remote_branch:$remote_oid" "$remote" "HEAD:$remote_branch"
else
  git push --force-with-lease="$remote_branch:" "$remote" "HEAD:$remote_branch"
fi

body="$(mktemp)"
trap 'rm -f "$msg" "$body"' EXIT
cat >"$body" <<EOF
## Buzz Desktop release v$version

- **Frozen main:** \`$base_sha\`
- **Reviewed candidate:** \`$candidate_sha\`
- **Previous desktop release:** \`$previous_tag\`
- **Proposed immutable tag:** \`desktop-v$version\`

This PR must be **squash merged** only after the Desktop Release Candidate check passes. The branch must remain based directly on current `main`; stale base, payload drift, incomplete notes, or an unauthorized merge produce no tag.

The checked-in changelog accounts for every non-merge commit in the release range. Publication remains bound to the immutable candidate tag.
EOF
if existing="$(gh pr list --head "$branch" --state open --json number --jq '.[0].number')" && [[ -n "$existing" ]]; then
  gh pr edit "$existing" --title "chore(release): release Buzz Desktop version $version" --body-file "$body"
else
  gh pr create --base main --head "$branch" \
    --title "chore(release): release Buzz Desktop version $version" --body-file "$body"
fi
