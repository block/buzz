#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "Usage: generate-oss-latest-json.sh <version> <platform-key:sig-file:archive-url>..." >&2
  echo "  e.g. generate-oss-latest-json.sh 1.2.3 \\" >&2
  echo "         darwin-aarch64:/path/to/app.sig:https://example.com/app.tar.gz \\" >&2
  echo "         windows-x86_64:/path/to/setup.sig:https://example.com/setup.exe" >&2
  exit 1
fi

VERSION="$1"
shift

# Build the jq `platforms` object from N triples. Each triple is
# `platform-key:sig-file:archive-url`; archive URLs contain colons, so split
# only on the first two so the URL stays intact.
platform_args=()
platforms_obj="{}"
i=0
for triple in "$@"; do
  key="${triple%%:*}"
  rest="${triple#*:}"
  sig_file="${rest%%:*}"
  url="${rest#*:}"
  if [[ "$key" == "$triple" || "$sig_file" == "$rest" || -z "$key" || -z "$sig_file" || -z "$url" ]]; then
    echo "Error: malformed triple '$triple' (expected platform-key:sig-file:archive-url)" >&2
    exit 1
  fi

  sig_arg="sig$i"
  url_arg="url$i"
  platform_args+=(--arg "$sig_arg" "$(cat "$sig_file")" --arg "$url_arg" "$url")
  # Splice each platform into the accumulating object so no platform is hardcoded.
  platforms_obj="$platforms_obj + { \"$key\": { signature: \$$sig_arg, url: \$$url_arg } }"
  i=$((i + 1))
done

jq -n \
  --arg version "$VERSION" \
  --arg notes "Buzz v$VERSION" \
  --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "${platform_args[@]}" \
  "{ version: \$version, notes: \$notes, pub_date: \$pub_date, platforms: ($platforms_obj) }"
