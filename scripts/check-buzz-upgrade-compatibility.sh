#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

target_ref="${1:-origin/main}"
git rev-parse --verify --quiet "${target_ref}^{commit}" >/dev/null || {
  echo "FAIL: cannot resolve Buzz target ref: ${target_ref}"
  exit 2
}

probe_dir="$(mktemp -d /tmp/buzz-upgrade-probe.XXXXXX)"
patch_file="$(mktemp /tmp/buzz-platform.XXXXXX.patch)"
index_file="$(mktemp /tmp/buzz-platform-index.XXXXXX)"

cleanup() {
  git worktree remove "$probe_dir" --force >/dev/null 2>&1 || true
  rm -f "$patch_file" "$index_file"
}
trap cleanup EXIT

source_base="$(git merge-base HEAD "$target_ref")"
if ! git merge-base --is-ancestor "$source_base" "$target_ref"; then
  echo "FAIL: ${target_ref} does not descend from source base ${source_base}."
  exit 2
fi

# Build the patch from an alternate index so committed, staged, unstaged, and
# untracked (non-ignored) product files are all covered without changing the
# developer's real index. This keeps the check meaningful after product work is
# committed and also catches a future upstream path collision with a product file.
rm -f "$index_file"
GIT_INDEX_FILE="$index_file" git read-tree HEAD
GIT_INDEX_FILE="$index_file" git add -A -- .
GIT_INDEX_FILE="$index_file" git diff --cached --binary "$source_base" >"$patch_file"

if [[ ! -s "$patch_file" ]]; then
  echo "PASS: no Business Platform changes relative to ${source_base}."
  exit 0
fi

git worktree add --detach "$probe_dir" "$target_ref" >/dev/null

if git -C "$probe_dir" apply --check "$patch_file"; then
  echo "PASS: full Business Platform patch applies cleanly to ${target_ref}."
  exit 0
fi

if git -C "$probe_dir" apply --3way "$patch_file" >/dev/null 2>&1; then
  echo "PASS: full Business Platform patch applies to ${target_ref} with Git three-way merge."
  exit 0
fi

conflicts="$(git -C "$probe_dir" diff --name-only --diff-filter=U)"
echo "FAIL: full Business Platform patch needs adaptation for ${target_ref}."
if [[ -n "$conflicts" ]]; then
  echo "Conflicting integration files:"
  echo "$conflicts"
fi
exit 1
