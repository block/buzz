#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${GITHUB_EVENT_PATH:-}" ]]; then
  echo "error: GITHUB_EVENT_PATH is required" >&2
  exit 1
fi

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
markdown_file=$(mktemp)
trap 'rm -f "$markdown_file"' EXIT

jq -er '
  if (.pull_request | type) != "object" then
    error("event does not contain a pull_request object")
  elif .pull_request.body == null then
    ""
  elif (.pull_request.body | type) == "string" then
    .pull_request.body
  else
    error("pull_request.body must be a string or null")
  end
' "$GITHUB_EVENT_PATH" >"$markdown_file"

"$script_dir/check-pr-image-urls.sh" "$markdown_file"
