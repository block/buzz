#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workflow="$root/.github/workflows/promote-oss-desktop-release.yml"
promoter="$root/scripts/promote-oss-desktop-release.sh"
release="$root/.github/workflows/release.yml"

# Pin the separation contract: tag builds retain the exact candidate but cannot
# mutate the rolling updater release.
grep -Fq 'cp latest.json staged/updater-manifest.json' "$release"
[[ "$(grep -c 'gh release upload' "$release")" -eq 1 ]]
! grep -Fq 'gh release upload buzz-desktop-latest' "$release"

grep -Fq 'workflow_dispatch:' "$workflow"
grep -Fq 'group: oss-desktop-auto-update-promotion' "$workflow"
grep -Fq 'cancel-in-progress: false' "$workflow"
grep -Fq 'if: github.repository ==' "$workflow"
grep -Fq 'DISPATCH_REF' "$workflow"
grep -Fq 'contents: write' "$workflow"
grep -Fq 'VERSION: ${{ inputs.version }}' "$workflow"
grep -Fq 'scripts/promote-oss-desktop-release.sh "$VERSION"' "$workflow"
if grep -F 'run:' "$workflow" | grep -Fq '${{ inputs.version }}'; then
  echo "untrusted workflow input must not be interpolated into run" >&2
  exit 1
fi

grep -Fq 'refusing downgrade' "$promoter"
grep -Fq 'current_digest="$(sha256sum "$current"' "$promoter"
grep -Fq '== "$current_digest"' "$promoter"
grep -Fq 'updater-manifest.json' "$promoter"
grep -Fq 'desktop-v" + $version + "/"' "$promoter"
grep -Fq 'gh release upload "$ROLLING_TAG" "$promotion"' "$promoter"
grep -Fq 'served latest.json does not match the promoted candidate' "$promoter"
grep -Fq 'promotion upload failed' "$promoter"

echo "OSS desktop promotion contract passed"
