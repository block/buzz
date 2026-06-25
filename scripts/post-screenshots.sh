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
COMMIT=$(git commit-tree "$TREE" "${PARENT_ARGS[@]}" -m "screenshots: PR #${PR}")
git push --force-with-lease origin "${COMMIT}:refs/heads/${BRANCH}"

RAW_BASE="https://raw.githubusercontent.com/${REPO}/${COMMIT}"

declare -A IMAGE_URL_MAP
IMAGE_URLS=()
for i in "${!PNGS[@]}"; do
  ORIG_NAME="$(basename "${PNGS[$i]}" .png)"
  URL="${RAW_BASE}/${TREE_PATHS[$i]}"
  IMAGE_URLS+=("$URL")
  IMAGE_URL_MAP["$ORIG_NAME"]="$URL"
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
