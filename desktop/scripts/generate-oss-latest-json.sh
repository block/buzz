#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 ]]; then
  echo "Usage: generate-oss-latest-json.sh <version> <sig-file> <archive-url>" >&2
  exit 1
fi

VERSION="$1"
SIG_FILE="$2"
ARCHIVE_URL="$3"

# Only darwin-aarch64 is included because the workflow builds on ARM64 runners
# only. Supporting Intel Macs (darwin-x86_64) would require a matrix build.
jq -n \
  --arg version "$VERSION" \
  --arg notes "Buzz v$VERSION" \
  --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg signature "$(cat "$SIG_FILE")" \
  --arg url "$ARCHIVE_URL" \
  '{ version: $version, notes: $notes, pub_date: $pub_date, platforms: { "darwin-aarch64": { signature: $signature, url: $url } } }'
