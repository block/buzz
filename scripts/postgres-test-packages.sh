#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_root="${1:-$repo_root/crates}"

exec python3 "$repo_root/scripts/check-postgres-test-discovery.py" \
  --print-packages "$source_root"
