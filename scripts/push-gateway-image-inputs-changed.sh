#!/usr/bin/env bash
# Print `true` when the push gateway image must be rebuilt for HEAD_REV, and
# `false` when its build inputs match PUBLISHED_REV, the source of the gateway
# image that main last published.
#
# Usage: scripts/push-gateway-image-inputs-changed.sh PUBLISHED_REV HEAD_REV
#
# The gateway crate depends on no other workspace crate, so only the paths
# below can change its image. Comparing against the published image, not the
# previous push, keeps a change from being lost when a main run is replaced
# while pending or fails. Any doubt builds: an empty or unknown revision, or
# one that is not an ancestor of HEAD_REV, prints `true`.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 PUBLISHED_REV HEAD_REV" >&2
  exit 2
fi
published_rev=$1
head_rev=$2

inputs=(
  crates/buzz-push-gateway
  Cargo.toml
  Cargo.lock
  .cargo
  Dockerfile.push-gateway
  .dockerignore
  .github/workflows/docker.yml
  scripts/push-gateway-image-inputs-changed.sh
)

if [[ ! "$published_rev" =~ ^[0-9a-f]{40}$ ]] ||
  ! git cat-file -e "${published_rev}^{commit}" 2>/dev/null ||
  ! git merge-base --is-ancestor "$published_rev" "$head_rev" 2>/dev/null; then
  echo true
  exit 0
fi

if git diff --quiet "$published_rev" "$head_rev" -- "${inputs[@]}"; then
  echo false
else
  echo true
fi
