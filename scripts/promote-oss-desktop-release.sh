#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-}"
REPOSITORY="${GITHUB_REPOSITORY:-block/buzz}"
TAG="desktop-v${VERSION}"
CANDIDATE="updater-manifest.json"
ROLLING_TAG="buzz-desktop-latest"
EXPECTED_PLATFORMS='["darwin-aarch64","darwin-x86_64","linux-x86_64","windows-x86_64"]'

fail() { echo "::error::$*" >&2; exit 1; }
[[ "$REPOSITORY" == "block/buzz" ]] || fail "promotion is restricted to block/buzz"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "version must be stable semver X.Y.Z"
command -v gh >/dev/null || fail "gh is required"
command -v jq >/dev/null || fail "jq is required"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
candidate="$workdir/$CANDIDATE"
current="$workdir/latest.json"

release_json="$(gh release view "$TAG" --repo "$REPOSITORY" --json isDraft,isPrerelease,targetCommitish,assets)"
[[ "$(jq -r .isDraft <<<"$release_json")" == false ]] || fail "$TAG is still a draft"
[[ "$(jq -r .isPrerelease <<<"$release_json")" == false ]] || fail "$TAG is a prerelease"

tag_sha="$(gh api "repos/$REPOSITORY/commits/$TAG" --jq .sha)"
target="$(jq -r .targetCommitish <<<"$release_json")"
target_sha="$(gh api "repos/$REPOSITORY/commits/$target" --jq .sha)"
[[ -n "$tag_sha" && "$target_sha" == "$tag_sha" ]] || fail "$TAG and its release target do not resolve to the same commit"

release_assets="$(jq -r '.assets[].name' <<<"$release_json")"
grep -Fxq "$CANDIDATE" <<<"$release_assets" || fail "$TAG has no $CANDIDATE asset"
gh release download "$TAG" --repo "$REPOSITORY" --pattern "$CANDIDATE" --dir "$workdir"

jq -e --arg version "$VERSION" --argjson expected "$EXPECTED_PLATFORMS" '
  .version == $version and
  (.platforms | keys == $expected) and
  ([.platforms[] | (.signature | type == "string" and length > 0)] | all) and
  ([.platforms[] | (.url | type == "string" and startswith("https://github.com/block/buzz/releases/download/desktop-v" + $version + "/"))] | all)
' "$candidate" >/dev/null || fail "$CANDIDATE failed version, platform, signature, or URL validation"

while IFS= read -r url; do
  asset="${url##*/}"
  [[ "$url" == "https://github.com/block/buzz/releases/download/$TAG/$asset" ]] || fail "$CANDIDATE contains non-canonical updater URL: $url"
  grep -Fxq "$asset" <<<"$release_assets" || fail "$CANDIDATE references missing release asset: $asset"
done < <(jq -r '.platforms[].url' "$candidate")

gh release download "$ROLLING_TAG" --repo "$REPOSITORY" --pattern latest.json --dir "$workdir"
current_digest="$(sha256sum "$current" | awk '{print $1}')"
current_version="$(jq -er '.version | select(type == "string")' "$current")" || fail "current latest.json has no version"
[[ "$current_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "current promoted version is not stable semver: $current_version"

highest="$(printf '%s\n%s\n' "$current_version" "$VERSION" | sort -V | tail -1)"
if [[ "$VERSION" == "$current_version" ]]; then
  cmp -s "$candidate" "$current" || fail "$VERSION is already promoted with different manifest content"
  echo "Version $VERSION is already promoted with identical manifest content."
  exit 0
fi
[[ "$highest" == "$VERSION" ]] || fail "refusing downgrade from $current_version to $VERSION"

# Re-read immediately before the only write so a stale validation cannot silently
# overwrite a promotion performed outside this workflow.
rm -f "$current"
gh release download "$ROLLING_TAG" --repo "$REPOSITORY" --pattern latest.json --dir "$workdir"
[[ "$(sha256sum "$current" | awk '{print $1}')" == "$current_digest" ]] || fail "current promotion changed during validation; retry"

promotion="$workdir/latest.json"
cp "$candidate" "$promotion"
candidate_digest="$(sha256sum "$candidate" | awk '{print $1}')"
if ! gh release upload "$ROLLING_TAG" "$promotion" --repo "$REPOSITORY" --clobber; then
  fail "promotion upload failed; latest.json may be temporarily unavailable, retry the promotion"
fi
rm -f "$promotion"
if ! gh release download "$ROLLING_TAG" --repo "$REPOSITORY" --pattern latest.json --dir "$workdir"; then
  fail "promotion upload returned success but latest.json could not be verified; retry the promotion"
fi
[[ "$(sha256sum "$promotion" | awk '{print $1}')" == "$candidate_digest" ]] || fail "served latest.json does not match the promoted candidate; retry the promotion"
{
  echo "### OSS desktop auto-update promoted"
  echo "- Version: \`$VERSION\`"
  echo "- Tag commit: \`$tag_sha\`"
  echo "- Previous version: \`$current_version\`"
  echo "- Manifest SHA-256: \`$(sha256sum "$candidate" | awk '{print $1}')\`"
  echo "- Actor: \`${GITHUB_ACTOR:-unknown}\`"
  if [[ -n "${GITHUB_SERVER_URL:-}" && -n "${GITHUB_RUN_ID:-}" ]]; then
    echo "- Workflow: ${GITHUB_SERVER_URL}/${REPOSITORY}/actions/runs/${GITHUB_RUN_ID}"
  fi
} >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
