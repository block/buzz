#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || -z "$1" ]]; then
  echo "usage: $0 <image-reference>" >&2
  exit 2
fi

# Distribution advisories without a vendor package cannot be remediated in the
# image. Fail as soon as a high- or critical-severity fix is available so a
# stale base image or package layer cannot continue to ship unnoticed.
bin/trivy image \
  --scanners vuln \
  --severity HIGH,CRITICAL \
  --ignore-unfixed \
  --exit-code 1 \
  "$1"
