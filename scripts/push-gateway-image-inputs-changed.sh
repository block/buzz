#!/usr/bin/env bash
# Print `build=true` when the push gateway image must be built for HEAD_REV,
# and `build=false` when its build inputs match the source of the current :main
# gateway image. docker.yml appends the line to $GITHUB_OUTPUT, so a failure
# here fails the step instead of leaving the decision empty.
#
# Usage: MAIN_PUSH=true|false scripts/push-gateway-image-inputs-changed.sh HEAD_REV
#
# Only a main push (MAIN_PUSH=true) may skip; every other event builds. The
# docker.yml merge job moves :main only after the image's provenance is
# attested, so its source label names a completed publication. Comparing with
# it, not the previous push, keeps a change from being lost when a main run is
# replaced while pending or fails. The gateway crate depends on no other
# workspace crate, so only the paths below can change its image. Any doubt
# builds: an unreadable image, or a source that is unknown or not an ancestor
# of HEAD_REV.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: MAIN_PUSH=true|false $0 HEAD_REV" >&2
  exit 2
fi
head_rev=$1

if [[ "${MAIN_PUSH:-}" != "true" ]]; then
  echo build=true
  exit 0
fi

inputs=(
  crates/buzz-push-gateway
  Cargo.toml
  Cargo.lock
  .cargo
  rust-toolchain.toml
  Dockerfile.push-gateway
  .dockerignore
  .github/workflows/docker.yml
  scripts/push-gateway-image-inputs-changed.sh
)

# A multi-platform index reports one image per platform; a single image
# reports its config directly.
published_rev=$(docker buildx imagetools inspect ghcr.io/block/buzz-push-gateway:main \
  --format '{{json .Image}}' 2>/dev/null |
  jq -r '(if has("config") then . else .["linux/amd64"] end)
    | .config.Labels["org.opencontainers.image.revision"] // empty' 2>/dev/null || true)
echo "Published :main gateway source: ${published_rev:-unknown}" >&2

if [[ ! "$published_rev" =~ ^[0-9a-f]{40}$ ]] ||
  ! git cat-file -e "${published_rev}^{commit}" 2>/dev/null ||
  ! git merge-base --is-ancestor "$published_rev" "$head_rev" 2>/dev/null; then
  echo build=true
  exit 0
fi

if git diff --quiet "$published_rev" "$head_rev" -- "${inputs[@]}"; then
  echo build=false
else
  echo build=true
fi
