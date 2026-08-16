#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/../.." && pwd)
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT

ln -s "$repo_root/scripts/tests/fixtures/offline-model-curl" "$test_dir/curl"
export PATH="$test_dir:$PATH"
export EXPECTED_MODEL="google/gemma-4-26b-a4b"
export EXPECTED_INSTANCE="gemma4-26b-official"

report="$test_dir/report.json"
"$repo_root/scripts/check-offline-model.sh" \
  --model "$EXPECTED_MODEL" \
  --instance "$EXPECTED_INSTANCE" \
  --report "$report" >/dev/null

jq -e --arg instance "$EXPECTED_INSTANCE" '
  .instanceId == $instance
  and .generationCapacity == 1
  and .reasoning == "off"
  and .result == "pass"
' "$report" >/dev/null
