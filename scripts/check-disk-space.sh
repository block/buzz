#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

case "${BUZZ_SKIP_DISK_PREFLIGHT:-}" in
  1|true|TRUE|yes|YES)
    echo "Disk preflight skipped (BUZZ_SKIP_DISK_PREFLIGHT=1)."
    exit 0
    ;;
esac

reserve_gib=${BUZZ_DISK_BUILD_RESERVE_GIB:-15}
min_free_gib=${BUZZ_DISK_MIN_FREE_GIB:-10}

if [[ ! "$reserve_gib" =~ ^[0-9]+$ ]]; then
  echo "BUZZ_DISK_BUILD_RESERVE_GIB must be a non-negative integer." >&2
  exit 2
fi
if [[ ! "$min_free_gib" =~ ^[0-9]+$ ]]; then
  echo "BUZZ_DISK_MIN_FREE_GIB must be a non-negative integer." >&2
  exit 2
fi

if ! available_kib=$(df -Pk "$repo_root" 2>/dev/null | awk 'NR == 2 { print $4 }'); then
  echo "Disk preflight warning: could not determine free space; continuing." >&2
  exit 0
fi
if [[ ! "${available_kib:-}" =~ ^[0-9]+$ ]]; then
  echo "Disk preflight warning: could not determine free space; continuing." >&2
  exit 0
fi

reserve_kib=$((reserve_gib * 1024 * 1024))
min_free_kib=$((min_free_gib * 1024 * 1024))
post_build_kib=$((available_kib - reserve_kib))

available_gib=$(awk -v kib="$available_kib" 'BEGIN { printf "%.1f", kib / 1048576 }')
post_build_gib=$(awk -v kib="$post_build_kib" 'BEGIN { printf "%.1f", kib / 1048576 }')

if ((post_build_kib < min_free_kib)); then
  cat >&2 <<EOF
Disk preflight blocked: ${available_gib} GiB is free now, but reserving ${reserve_gib} GiB for Buzz's local checks would leave ${post_build_gib} GiB.
The configured minimum is ${min_free_gib} GiB. Free disk space before pushing, or adjust BUZZ_DISK_BUILD_RESERVE_GIB / BUZZ_DISK_MIN_FREE_GIB if this checkout is already warm.
To bypass only this guard while keeping the other hooks, set BUZZ_SKIP_DISK_PREFLIGHT=1.
EOF
  exit 1
fi

echo "Disk preflight passed: ${available_gib} GiB free; estimated post-check free space ${post_build_gib} GiB (minimum ${min_free_gib} GiB)."
