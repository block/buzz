#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  printf 'usage: %s <previous-build-sha> <candidate-sha> <branch>\n' "$0" >&2
  exit 2
fi

previous_sha=$1
candidate_sha=$2
branch=$3
canonical_branch=windows

if [[ "$branch" != "$canonical_branch" ]]; then
  printf 'Windows builds are allowed only from %s; got %s\n' \
    "$canonical_branch" "$branch" >&2
  exit 1
fi

for commit in "$previous_sha" "$candidate_sha"; do
  if ! git cat-file -e "${commit}^{commit}" 2>/dev/null; then
    printf 'Windows lineage commit is unavailable: %s\n' "$commit" >&2
    exit 1
  fi
done

if ! git merge-base --is-ancestor "$previous_sha" "$candidate_sha"; then
  printf 'Windows candidate %s does not contain previous build %s\n' \
    "$candidate_sha" "$previous_sha" >&2
  exit 1
fi

printf 'windows client lineage: OK (%s -> %s on %s)\n' \
  "$previous_sha" "$candidate_sha" "$branch"
