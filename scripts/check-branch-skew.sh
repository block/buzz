#!/usr/bin/env bash
# Pre-push guard: CI checks the PR merged with main, so local runs on a
# skewed branch can pass while CI fails. Block the push only when
# origin/main has changed files this branch also touches.
set -euo pipefail

branch=$(git rev-parse --abbrev-ref HEAD)
if [ "$branch" = "main" ] || [ "$branch" = "HEAD" ]; then
  exit 0
fi

git fetch --quiet origin main || true
git rev-parse --verify --quiet origin/main >/dev/null || exit 0

base=$(git merge-base HEAD origin/main)
if [ "$base" = "$(git rev-parse origin/main)" ]; then
  exit 0
fi

overlap=$(comm -12 \
  <(git diff --name-only "$base" origin/main -- | sort) \
  <(git diff --name-only "$base" HEAD -- | sort))

if [ -z "$overlap" ]; then
  exit 0
fi

{
  echo "Branch is behind origin/main, and main changed files this branch also touches:"
  echo "$overlap" | sed 's/^/  /'
  echo "Local checks ran on a tree CI will never test. Run 'git merge origin/main',"
  echo "resolve, re-run checks, then push."
} >&2
exit 1
