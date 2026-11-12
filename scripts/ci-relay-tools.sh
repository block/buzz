#!/usr/bin/env bash
# Linux x86_64 relay-test setup. Publish tools only after every fetch succeeds.
set -euo pipefail

tools="$RUNNER_TEMP/buzz-relay-tools"
mkdir -p "$tools"

fetch() {
  local url=$1 digest=$2 destination=$3 nonce
  nonce=$(mktemp -d "$tools/fetch.XXXXXXXXXXXX")
  # A fresh URL avoids reusing a cached failed GitHub release redirect.
  curl -fsSL --connect-timeout 15 --max-time 60 \
    "$url?download=1&ci-attempt=${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-0}-${nonce##*/}" \
    -o "$destination.download"
  printf '%s  %s\n' "$digest" "$destination.download" | shasum -a 256 --check
  mv "$destination.download" "$destination"
}

# This is the same artifact currently published on Hermit's stable channel.
fetch "https://github.com/cashapp/hermit/releases/download/v0.55.1/hermit-linux-amd64.gz" \
  ef781d50bbfa57c8b0f03bb7f442c953a00a5726e9758d19f6c549ffd3e6e676 "$tools/hermit.gz"
gzip -dc "$tools/hermit.gz" > "$tools/hermit.tmp"
chmod +x "$tools/hermit.tmp"
mv "$tools/hermit.tmp" "$tools/hermit"
export HERMIT_EXE="$tools/hermit"
export HERMIT_SELF_UPDATE=false

# Read the version, URL and checksum from the repository's existing Hermit pin.
metadata=$(./bin/hermit info --json pgschema)
source_url=$(printf '%s' "$metadata" | jq -er '.[0].Source')
digest=$(printf '%s' "$metadata" | jq -er '.[0].SHA256')
fetch "$source_url" "$digest" "$tools/pgschema"
chmod +x "$tools/pgschema"
export PGSCHEMA_BIN="$tools/pgschema"
"$PGSCHEMA_BIN" --help > /dev/null

./bin/hermit env --raw > "$tools/environment"
{
  printf 'HERMIT_EXE=%s\nHERMIT_SELF_UPDATE=false\nPGSCHEMA_BIN=%s\n' "$HERMIT_EXE" "$PGSCHEMA_BIN"
  cat "$tools/environment"
} >> "$GITHUB_ENV"
