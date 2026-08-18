#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 <markdown-file>" >&2
  exit 1
fi

MARKDOWN_FILE="$1"

if [[ ! -f "$MARKDOWN_FILE" ]]; then
  echo "error: markdown file not found: $MARKDOWN_FILE" >&2
  exit 1
fi

# Buzz/relay media URLs often render in Buzz but fail in GitHub PR markdown
# because GitHub's Camo proxy fetches them anonymously. PR screenshots should
# be hosted through scripts/post-screenshots.sh or another GitHub-safe host.
relay_media_pattern='https?://[^][()<>[:space:]"'"'"']*/media/[0-9a-fA-F]{64}\.(png|jpe?g|webp|gif)'
sprout_media_pattern='https?://sprout-oss[^][()<>[:space:]"'"'"']*/media/'

tmp_matches=$(mktemp)
trap 'rm -f "$tmp_matches"' EXIT

if grep -nE "$relay_media_pattern" "$MARKDOWN_FILE" >>"$tmp_matches"; then
  :
fi
if grep -nE "$sprout_media_pattern" "$MARKDOWN_FILE" >>"$tmp_matches"; then
  :
fi

if [[ -s "$tmp_matches" ]]; then
  matches=$(sort -u "$tmp_matches")
  echo "error: PR markdown contains Buzz/relay media URLs that may not render on GitHub:" >&2
  printf '%s\n' "$matches" >&2
  echo >&2
  echo "Upload screenshots with scripts/post-screenshots.sh, then use its GitHub-safe image URLs in the PR body/comment." >&2
  exit 1
fi
