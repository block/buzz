#!/usr/bin/env bash
# Publish a manually selected relay source without CI qualification.
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
exec python3 "$script_dir/manual_relay_image.py" build "$@"
