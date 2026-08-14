#!/usr/bin/env bash
# Pre-push guard, cohort variant of scripts/check-branch-skew.sh (#15).
#
# The upstream script assumes `origin` is the project and `main` is the PR base —
# both true in block/buzz, neither true here. This fork's PR base is `launchpad`,
# and which remote name resolves to the cohort repo varies by contributor: some
# have `origin` pointing straight at launchpad-26/buzz (so `origin/launchpad`
# exists), others keep `origin` as a personal fork and add a separate `launchpad`
# remote (so it's `launchpad/launchpad` that exists, and `origin/launchpad` does
# not). Hardcoding either shape blocks the other contributor's valid pushes, so
# this searches every configured remote for a ref literally named `launchpad` and
# uses whichever one resolves first.
#
# Lives under launchpad/ rather than editing scripts/check-branch-skew.sh directly
# (see launchpad/AGENTS.md §3): the upstream script is unmodified and still
# reusable, and this file carries no upstream merge-conflict risk. lefthook.yml's
# own `run:` line for pre-push.branch-skew points here instead of at the
# upstream path — that's the only edit made to an upstream-owned file.
set -euo pipefail

branch=$(git rev-parse --abbrev-ref HEAD)
if [ "$branch" = "launchpad" ] || [ "$branch" = "HEAD" ]; then
  exit 0
fi

base_ref=""
for remote in $(git remote); do
  git fetch --quiet "$remote" launchpad 2>/dev/null || continue
  if git rev-parse --verify --quiet "${remote}/launchpad" >/dev/null; then
    base_ref="${remote}/launchpad"
    break
  fi
done

if [ -z "$base_ref" ]; then
  # No remote has a `launchpad` ref at all — cannot compare, so do not block.
  # Matches the upstream script's own fallback when origin/main is unresolvable.
  exit 0
fi

base=$(git merge-base HEAD "$base_ref")
if [ "$base" = "$(git rev-parse "$base_ref")" ]; then
  exit 0
fi

overlap=$(comm -12 \
  <(git diff --name-only "$base" "$base_ref" -- | sort) \
  <(git diff --name-only "$base" HEAD -- | sort))

if [ -z "$overlap" ]; then
  exit 0
fi

{
  echo "Branch is behind ${base_ref}, and launchpad changed files this branch also touches:"
  echo "$overlap" | sed 's/^/  /'
  echo "Local checks ran on a tree CI will never test. Run 'git merge ${base_ref}',"
  echo "resolve, re-run checks, then push."
} >&2
exit 1
