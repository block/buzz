#!/usr/bin/env bash
# Pre-push guard: CI checks the PR merged with its base. Use gh's configured
# PR base for stacked branches, defaulting to main. Block when that base has
# changed files this branch also touches.
set -euo pipefail

branch=$(git rev-parse --abbrev-ref HEAD)
if [ "$branch" = "main" ] || [ "$branch" = "HEAD" ]; then
  exit 0
fi

target=$(git config --get "branch.$branch.gh-merge-base") || target=main
if ! git check-ref-format "refs/heads/$target" || [ "$target" = "$branch" ]; then
  echo "Invalid PR base for $branch: '$target'" >&2
  exit 1
fi
target_ref="refs/remotes/origin/$target"
git fetch --quiet origin "+refs/heads/$target:$target_ref"
target_oid=$(git rev-parse --verify "$target_ref^{commit}")

base=$(git merge-base HEAD "$target_oid")
if [ "$base" = "$target_oid" ]; then
  exit 0
fi

overlap=$(comm -12 \
  <(git diff --name-only "$base" "$target_oid" -- | sort) \
  <(git diff --name-only "$base" HEAD -- | sort))

if [ -z "$overlap" ]; then
  exit 0
fi

{
  echo "Branch is behind origin/$target, which changed files this branch also touches:"
  echo "$overlap" | sed 's/^/  /'
  echo "Local checks ran on a tree CI will never test. Run 'git merge origin/$target',"
  echo "resolve, re-run checks, then push."
} >&2
exit 1
