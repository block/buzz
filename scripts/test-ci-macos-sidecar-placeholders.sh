#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workflow="$repo_root/.github/workflows/ci.yml"

macos_job=$(
  awk '
    /^  desktop-build-macos:$/ { in_job = 1 }
    in_job && /^  [[:alnum:]_-]+:$/ && $0 !~ /^  desktop-build-macos:$/ { exit }
    in_job { print }
  ' "$workflow"
)

for sidecar in \
  buzz-acp \
  buzz-agent \
  buzz-lmstudio-agent \
  buzz-backend-kubernetes \
  buzz-dev-mcp \
  git-credential-nostr \
  buzz \
  buzz-apple-inputs; do
  if ! grep -Fq "desktop/src-tauri/binaries/$sidecar-\$TARGET" <<<"$macos_job"; then
    echo "Desktop Build (macOS) does not create the required $sidecar placeholder" >&2
    exit 1
  fi
done

echo "macOS CI sidecar placeholder contract passed"
