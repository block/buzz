#!/usr/bin/env bash
# Sync radu2lupu/buzz with block/buzz while keeping our patch branch on top.
#
# Usage:
#   ./scripts/sync-upstream.sh           # fetch + rebase only
#   ./scripts/sync-upstream.sh --push    # also push main + force-with-lease patch branch
#
# Patch branch: agent/native-grok-acp (Grok ACP + thread participation + fieldcraft env)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PATCH_BRANCH="${BUZZ_FORK_PATCH_BRANCH:-agent/native-grok-acp}"
UPSTREAM_REMOTE="${BUZZ_UPSTREAM_REMOTE:-upstream}"
ORIGIN_REMOTE="${BUZZ_ORIGIN_REMOTE:-origin}"
PUSH=false
for arg in "$@"; do
  case "$arg" in
    --push) PUSH=true ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
  esac
done

if ! git remote get-url "$UPSTREAM_REMOTE" >/dev/null 2>&1; then
  git remote add "$UPSTREAM_REMOTE" https://github.com/block/buzz.git
fi

echo "==> fetch $UPSTREAM_REMOTE/main + $ORIGIN_REMOTE"
git fetch "$UPSTREAM_REMOTE" main
git fetch "$ORIGIN_REMOTE" 2>/dev/null || true

if ! git show-ref --verify --quiet "refs/heads/$PATCH_BRANCH"; then
  echo "error: patch branch '$PATCH_BRANCH' not found" >&2
  exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "error: working tree dirty — commit or stash before sync" >&2
  git status -sb
  exit 1
fi

CURRENT="$(git rev-parse --abbrev-ref HEAD)"
UPSTREAM_TIP="$(git rev-parse "$UPSTREAM_REMOTE/main")"
echo "==> upstream/main = $(git rev-parse --short "$UPSTREAM_TIP") $(git log -1 --oneline "$UPSTREAM_TIP")"

echo "==> reset local main -> $UPSTREAM_REMOTE/main"
git checkout main
git reset --hard "$UPSTREAM_TIP"

echo "==> rebase $PATCH_BRANCH onto main"
git checkout "$PATCH_BRANCH"
if ! git rebase main; then
  echo ""
  echo "REBASE STOPPED on conflicts. Fix files, then:"
  echo "  git add -A && GIT_EDITOR=true git rebase --continue"
  echo "  # or: git rebase --abort"
  echo "After a clean rebase: ./scripts/install-participation-acp.sh"
  exit 1
fi

echo "==> patch stack on main:"
git log --oneline main..HEAD

if $PUSH; then
  echo "==> push main + $PATCH_BRANCH (force-with-lease)"
  git push "$ORIGIN_REMOTE" main
  git push --force-with-lease "$ORIGIN_REMOTE" "$PATCH_BRANCH"
fi

if [[ "$CURRENT" != "$PATCH_BRANCH" && "$CURRENT" != "HEAD" ]]; then
  git checkout "$CURRENT" 2>/dev/null || true
fi

echo ""
echo "Sync ok. Next: ./scripts/install-participation-acp.sh && restart Buzz."
