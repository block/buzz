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
REPO="block/sprout"
RAW_BASE="https://raw.githubusercontent.com/${REPO}/refs/heads/${BRANCH}"

mapfile -t PNGS < <(find "$PNG_DIR" -maxdepth 1 -name "*.png" -type f | sort)
if [[ ${#PNGS[@]} -eq 0 ]]; then
  echo "error: no PNGs found in $PNG_DIR" >&2
  exit 1
fi

EXISTING_ENTRIES=""
if git fetch origin "refs/heads/${BRANCH}:refs/remotes/origin/${BRANCH}" 2>/dev/null; then
  EXISTING_ENTRIES=$(git ls-tree "origin/${BRANCH}" | grep -v $'\t'"\"\\{0,1\\}pr-${PR}--" || true)
fi

NEW_ENTRIES=""
IMAGE_URLS=()
for PNG in "${PNGS[@]}"; do
  FILENAME=$(basename "$PNG")
  if ! [[ "$FILENAME" =~ ^[a-zA-Z0-9_.-]+$ ]]; then
    echo "error: invalid PNG filename (must be alphanumeric, dots, hyphens, underscores): $FILENAME" >&2
    exit 1
  fi
  BLOB=$(git hash-object -w "$PNG")
  TREE_PATH="pr-${PR}--${FILENAME}"
  NEW_ENTRIES+="$(printf '100644 blob %s\t%s' "$BLOB" "$TREE_PATH")"$'\n'
  IMAGE_URLS+=("${RAW_BASE}/${TREE_PATH}")
done

COMBINED=$(printf '%s\n' "$EXISTING_ENTRIES" "$NEW_ENTRIES" | grep -v '^$')
TREE=$(echo "$COMBINED" | git mktree)

COMMIT=$(git commit-tree "$TREE" -m "screenshots: PR #${PR}")
git push --force-with-lease origin "${COMMIT}:refs/heads/${BRANCH}"

declare -A IMAGE_URL_MAP
for i in "${!PNGS[@]}"; do
  ORIG_NAME="$(basename "${PNGS[$i]}" .png)"
  IMAGE_URL_MAP["$ORIG_NAME"]="${IMAGE_URLS[$i]}"
done

if [[ -n "$BODY_FILE" ]]; then
  COMMENT_BODY="$(cat "$BODY_FILE")"
  UNREFERENCED=()
  for NAME in "${!IMAGE_URL_MAP[@]}"; do
    URL="${IMAGE_URL_MAP[$NAME]}"
    PLACEHOLDER="{{${NAME}}}"
    if [[ "$COMMENT_BODY" == *"$PLACEHOLDER"* ]]; then
      COMMENT_BODY="${COMMENT_BODY//"$PLACEHOLDER"/![$NAME]($URL)}"
    else
      UNREFERENCED+=("$NAME")
    fi
  done
  if [[ ${#UNREFERENCED[@]} -gt 0 ]]; then
    IFS=$'\n' SORTED=($(printf '%s\n' "${UNREFERENCED[@]}" | sort)); unset IFS
    for NAME in "${SORTED[@]}"; do
      COMMENT_BODY+=$'\n\n'"![${NAME}](${IMAGE_URL_MAP[$NAME]})"
    done
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
