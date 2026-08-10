#!/usr/bin/env bash
# Resolve the ghcr.io/block/buzz image tag that matches the deploy/compose bundle
# in a launchpad fork checkout.
#
# Why this exists: the relay runs from a prebuilt upstream image, but the compose
# bundle is deployed from the fork. Pinning to whatever upstream built most
# recently pairs new relay code with older config. The correct pin is the upstream
# commit the fork is synced to — and this refuses to answer if the fork has local
# edits to the bundle, because then no upstream image matches what you are running.
#
#   ./resolve-image-tag.sh [repo-path] [ref]
#
# Prints a BUZZ_IMAGE= line for deploy/compose/.env. Exit 1 if no honest pin exists.
set -euo pipefail

REPO="${1:-$HOME/group-build-project/buzz}"
REF="${2:-HEAD}"
BUNDLE="deploy/compose"
IMAGE="ghcr.io/block/buzz"
UPSTREAM_URL="https://github.com/block/buzz.git"
MAX_WALK=40

die() { printf '\nERROR: %s\n' "$1" >&2; exit 1; }
note() { printf '%s\n' "$1" >&2; }

cd "$REPO" 2>/dev/null || die "not a directory: $REPO"
git rev-parse --git-dir >/dev/null 2>&1 || die "not a git repo: $REPO"

# The commit graph is enough; blobs are not needed for merge-base or diffing paths
# we already have locally, so avoid dragging down a second copy of the tree.
if ! git remote get-url upstream >/dev/null 2>&1; then
  note "adding upstream remote -> $UPSTREAM_URL"
  git remote add upstream "$UPSTREAM_URL"
fi
note "fetching upstream/main (commits only)..."
git fetch upstream main --filter=blob:none --quiet

SYNC=$(git merge-base "$REF" upstream/main) || die "no common ancestor between $REF and upstream/main"
note "sync point:  $(git log -1 --format='%h %ad %s' --date=short "$SYNC")"

# An honest pin requires the bundle we deploy to be the bundle upstream built at
# that commit. launchpad/AGENTS.md section 3 forbids editing deploy/compose/, so a
# difference here means either a stray local edit or a divergent sync.
if ! git diff --quiet "$SYNC" "$REF" -- "$BUNDLE"; then
  note ""
  note "$BUNDLE differs between $REF and the sync point:"
  git diff --stat "$SYNC" "$REF" -- "$BUNDLE" >&2
  die "no upstream image matches this bundle. Revert local edits to $BUNDLE, or deploy from a synced ref."
fi
note "bundle check: $BUNDLE at $REF is identical to the sync point"

TOKEN=$(curl -fsS "https://ghcr.io/token?scope=repository:block/buzz:pull&service=ghcr.io" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])') \
  || die "could not obtain an anonymous ghcr.io pull token"

# HEAD the manifest rather than GET: we only need existence and the digest.
manifest_digest() {
  curl -fsS -o /dev/null -D - \
    -H "Authorization: Bearer $TOKEN" \
    -H "Accept: application/vnd.oci.image.index.v1+json,application/vnd.docker.distribution.manifest.list.v2+json,application/vnd.docker.distribution.manifest.v2+json" \
    "https://ghcr.io/v2/block/buzz/manifests/$1" 2>/dev/null \
    | grep -i '^docker-content-digest' | tr -d '\r' | awk '{print $2}'
}

# Not every upstream commit publishes an image. Walk back along first-parent from
# the sync point, but only accept a candidate whose bundle is also unchanged
# relative to the sync point — otherwise the older image expects different config.
note "searching for a published image at or before the sync point..."
FOUND="" ; DIGEST="" ; WALKED=0
while read -r commit; do
  WALKED=$((WALKED + 1))
  short=$(git rev-parse --short=7 "$commit")
  if ! git diff --quiet "$commit" "$SYNC" -- "$BUNDLE"; then
    note "  sha-$short  skipped: bundle differs from sync point"
    continue
  fi
  d=$(manifest_digest "sha-$short" || true)
  if [ -n "$d" ]; then
    FOUND="sha-$short" ; DIGEST="$d"
    break
  fi
done < <(git rev-list --first-parent -n "$MAX_WALK" "$SYNC")

[ -n "$FOUND" ] || die "no published image found in the $WALKED commits at or before the sync point"

if [ "$FOUND" != "sha-$(git rev-parse --short=7 "$SYNC")" ]; then
  note "note: the sync point itself has no published image; using the nearest older"
  note "      commit whose bundle is identical ($FOUND)"
fi

cat <<EOF

# Resolved $(date -u +%Y-%m-%dT%H:%M:%SZ) by scripts/resolve-image-tag.sh
# fork ref:   $REF ($(git rev-parse --short=9 "$REF"))
# sync point: $(git rev-parse --short=9 "$SYNC")
# digest:     $DIGEST
BUZZ_IMAGE=$IMAGE:$FOUND
EOF
