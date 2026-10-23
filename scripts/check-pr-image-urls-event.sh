#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${GITHUB_EVENT_PATH:-}" ]]; then
  echo "error: GITHUB_EVENT_PATH is required" >&2
  exit 1
fi

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
markdown_file=$(mktemp)
trap 'rm -f "$markdown_file"' EXIT

if ! jq -e '(.pull_request | type) == "object"' "$GITHUB_EVENT_PATH" \
  >/dev/null 2>&1; then
  echo "error: event does not contain a pull_request object" >&2
  exit 1
fi

jq -er '
  if .pull_request.body == null then
    ""
  elif (.pull_request.body | type) == "string" then
    .pull_request.body
  else
    error("pull_request.body must be a string or null")
  end
' "$GITHUB_EVENT_PATH" >"$markdown_file"

"$script_dir/check-pr-image-urls.sh" "$markdown_file"
