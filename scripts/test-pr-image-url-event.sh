#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
checker="$repo_root/scripts/check-pr-image-urls-event.sh"
test_tmp=$(mktemp -d)
trap 'rm -rf "$test_tmp"' EXIT

bad_url="https://buzz.example.com/media/$(printf 'a%.0s' {1..64}).png"
bad_body="Screenshot: ![broken]($bad_url)"$'\n'"\$(touch \"$test_tmp/injected\")"
jq -n --arg body "$bad_body" \
  '{pull_request: {body: $body}}' >"$test_tmp/bad-event.json"

if GITHUB_EVENT_PATH="$test_tmp/bad-event.json" "$checker" \
  >"$test_tmp/bad-output" 2>&1; then
  echo "PR event checker accepted a relay media URL" >&2
  exit 1
fi
grep -Fq "$bad_url" "$test_tmp/bad-output"
grep -Fq "scripts/post-screenshots.sh" "$test_tmp/bad-output"
[[ ! -e "$test_tmp/injected" ]]

jq -n --arg body 'Screenshot: ![safe](https://github.com/user-attachments/assets/example)' \
  '{pull_request: {body: $body}}' >"$test_tmp/good-event.json"
GITHUB_EVENT_PATH="$test_tmp/good-event.json" "$checker"

jq -n '{pull_request: {body: null}}' >"$test_tmp/empty-event.json"
GITHUB_EVENT_PATH="$test_tmp/empty-event.json" "$checker"

jq -n '{workflow_dispatch: {}}' >"$test_tmp/non-pr-event.json"
if GITHUB_EVENT_PATH="$test_tmp/non-pr-event.json" "$checker" \
  >"$test_tmp/non-pr-output" 2>&1; then
  echo "PR event checker accepted an event without a pull_request object" >&2
  exit 1
fi
expected_non_pr_error="error: event does not contain a pull_request object"
if [[ "$(<"$test_tmp/non-pr-output")" != "$expected_non_pr_error" ]]; then
  echo "PR event checker did not emit the expected non-PR event error" >&2
  exit 1
fi

echo "PR image URL event test passed"
