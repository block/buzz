#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workflow="$repo_root/.github/workflows/signed-macos-canary.yml"

grep -Fq 'workflow_dispatch:' "$workflow"
# The literal GitHub expression is the contract we are checking.
# shellcheck disable=SC2016
grep -Fq 'SOURCE_REF: ${{ github.ref }}' "$workflow"
grep -Fq '"refs/heads/main"' "$workflow"
grep -Fq 'contents: read' "$workflow"
grep -Fq 'id-token: write' "$workflow"
grep -Fq 'block/apple-codesign-action@' "$workflow"
grep -Fq 'actions/upload-artifact@' "$workflow"
grep -Fq 'retention-days: 7' "$workflow"
grep -Fq '"createUpdaterArtifacts": false' "$workflow"

if grep -Eq 'contents: write|gh release|buzz-desktop-latest|latest\.json|TAURI_SIGNING_PRIVATE_KEY|verify-release-ref\.sh|refs/tags/' "$workflow"; then
  echo "signed canary workflow gained a release or publishing capability" >&2
  exit 1
fi

on_block=$(
  awk '
    /^on:$/ { in_on = 1; next }
    in_on && /^[^[:space:]#]/ { exit }
    in_on && NF && $0 !~ /^[[:space:]]*#/ {
      gsub(/[[:space:]]/, "")
      print
    }
  ' "$workflow"
)
if [[ "$on_block" != "workflow_dispatch:" ]]; then
  echo "signed canary workflow must have workflow_dispatch as its only trigger" >&2
  printf 'found on block:\n%s\n' "$on_block" >&2
  exit 1
fi

echo "signed canary contract passed"
