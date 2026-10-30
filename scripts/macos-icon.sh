#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Hermit pins uv; inline script metadata pins Python and Pillow exactly.
# uv script environments are isolated from the caller's Python packages.
exec "$ROOT/bin/uv" run --managed-python --python 3.14.3 \
  --script "$ROOT/scripts/generate-macos-icon.py" "$@"
