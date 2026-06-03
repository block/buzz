#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "Usage: $0 <pr-number> <png-dir> [comment-body-file]" >&2
  exit 1
fi

PR="$1"
PNG_DIR="$2"
BODY_FILE="${3:-}"

BRANCH="agent-screenshots"
REPO="block/sprout"
RAW_BASE="https://raw.githubusercontent.com/${REPO}/refs/heads/${BRANCH}"

mapfile -t PNGS < <(find "$PNG_DIR" -maxdepth 1 -name "*.png" -type f | sort)
if [[ ${#PNGS[@]} -eq 0 ]]; then
  echo "error: no PNGs found in $PNG_DIR" >&2
  exit 1
fi

EXISTING_ENTRIES=""
if git fetch origin "refs/heads/${BRANCH}:refs/remotes/origin/${BRANCH}" 2>/dev/null; then
  EXISTING_ENTRIES=$(git ls-tree "origin/${BRANCH}" | grep -v "	pr-${PR}/" || true)
fi

NEW_ENTRIES=""
IMAGE_URLS=()
for PNG in "${PNGS[@]}"; do
  FILENAME=$(basename "$PNG")
  BLOB=$(git hash-object -w "$PNG")
  TREE_PATH="pr-${PR}/${FILENAME}"
  NEW_ENTRIES+="100644 blob ${BLOB}	${TREE_PATH}"$'\n'
  IMAGE_URLS+=("${RAW_BASE}/${TREE_PATH}")
done

COMBINED=$(printf '%s\n' "$EXISTING_ENTRIES" "$NEW_ENTRIES" | grep -v '^$')
TREE=$(echo "$COMBINED" | git mktree)

COMMIT=$(git commit-tree "$TREE" -m "screenshots: PR #${PR}")
git push --force origin "${COMMIT}:refs/heads/${BRANCH}"

if [[ -n "$BODY_FILE" ]]; then
  COMMENT_BODY=$(cat "$BODY_FILE")
  COMMENT_BODY+=$'\n\n'
  for URL in "${IMAGE_URLS[@]}"; do
    FILENAME=$(basename "$URL")
    NAME="${FILENAME%.png}"
    COMMENT_BODY+="![${NAME}](${URL})"$'\n\n'
  done
else
  COMMENT_BODY="## Screenshots"$'\n\n'
  for URL in "${IMAGE_URLS[@]}"; do
    FILENAME=$(basename "$URL")
    NAME="${FILENAME%.png}"
    COMMENT_BODY+="![${NAME}](${URL})"$'\n\n'
  done
fi

gh pr comment "$PR" --repo "$REPO" --body "$COMMENT_BODY"
echo "Posted ${#PNGS[@]} screenshot(s) to PR #${PR}"
