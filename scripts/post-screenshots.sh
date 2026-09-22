#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "Usage: $0 <pr-number> <png-dir> [comment-body-file]" >&2
  exit 1
fi

PR="$1"
PNG_DIR="$2"
BODY_FILE="${3:-}"

if ! [[ "$PR" =~ ^[0-9]+$ ]]; then
  echo "error: PR number must be a positive integer" >&2
  exit 1
fi

GH_USER=$(gh api user --jq .login)
BRANCH="agent-screenshots/${GH_USER}"
REPO="block/buzz"

# macOS ships bash 3.2, which lacks mapfile — build the array with read.
PNGS=()
while IFS= read -r PNG; do
  PNGS+=("$PNG")
done < <(find "$PNG_DIR" -maxdepth 1 -name "*.png" -type f | sort)
if [[ ${#PNGS[@]} -eq 0 ]]; then
  echo "error: no PNGs found in $PNG_DIR" >&2
  exit 1
fi

EXISTING_ENTRIES=""
if git fetch origin "refs/heads/${BRANCH}:refs/remotes/origin/${BRANCH}" 2>/dev/null; then
  EXISTING_ENTRIES=$(git ls-tree "origin/${BRANCH}" | grep -v $'\t'"\"\\{0,1\\}pr-${PR}--" || true)
fi

NEW_ENTRIES=""
TREE_PATHS=()
for PNG in "${PNGS[@]}"; do
  FILENAME=$(basename "$PNG")
  if ! [[ "$FILENAME" =~ ^[a-zA-Z0-9_.-]+$ ]]; then
    echo "error: invalid PNG filename (must be alphanumeric, dots, hyphens, underscores): $FILENAME" >&2
    exit 1
  fi
  BLOB=$(git hash-object -w "$PNG")
  TREE_PATH="pr-${PR}--${FILENAME}"
  NEW_ENTRIES+="$(printf '100644 blob %s\t%s' "$BLOB" "$TREE_PATH")"$'\n'
  TREE_PATHS+=("$TREE_PATH")
done

COMBINED=$(printf '%s\n' "$EXISTING_ENTRIES" "$NEW_ENTRIES" | grep -v '^$')
TREE=$(echo "$COMBINED" | git mktree)

PARENT_ARGS=()
if git rev-parse "origin/${BRANCH}" >/dev/null 2>&1; then
  PARENT_ARGS=(-p "origin/${BRANCH}")
fi
# ${arr[@]+...} guards the empty-array case, which trips set -u on bash 3.2.
COMMIT=$(git commit-tree "$TREE" ${PARENT_ARGS[@]+"${PARENT_ARGS[@]}"} -m "screenshots: PR #${PR}")
git push --force-with-lease origin "${COMMIT}:refs/heads/${BRANCH}"

RAW_BASE="https://raw.githubusercontent.com/${REPO}/${COMMIT}"

# Parallel name/url arrays instead of an associative array (bash 4+ only).
IMAGE_NAMES=()
IMAGE_URLS=()
for i in "${!PNGS[@]}"; do
  IMAGE_NAMES+=("$(basename "${PNGS[$i]}" .png)")
  IMAGE_URLS+=("${RAW_BASE}/${TREE_PATHS[$i]}")
done

if [[ -n "$BODY_FILE" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  "$SCRIPT_DIR/check-pr-image-urls.sh" "$BODY_FILE"
  COMMENT_BODY="$(cat "$BODY_FILE")"
  UNREFERENCED=()
  for i in "${!IMAGE_NAMES[@]}"; do
    NAME="${IMAGE_NAMES[$i]}"
    URL="${IMAGE_URLS[$i]}"
    PLACEHOLDER="{{${NAME}}}"
    if [[ "$COMMENT_BODY" == *"$PLACEHOLDER"* ]]; then
      COMMENT_BODY="${COMMENT_BODY//"$PLACEHOLDER"/![$NAME]($URL)}"
    else
      UNREFERENCED+=("${NAME}"$'\t'"${URL}")
    fi
  done
  if [[ ${#UNREFERENCED[@]} -gt 0 ]]; then
    while IFS=$'\t' read -r NAME URL; do
      COMMENT_BODY+=$'\n\n'"![${NAME}](${URL})"
    done < <(printf '%s\n' "${UNREFERENCED[@]}" | sort)
  fi
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
