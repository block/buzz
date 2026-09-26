#!/usr/bin/env bash
# Run against both the staged binary and the signed .app executable.
set -euo pipefail
BINARY=${1:?usage: verify-bundled-goose.sh /path/to/goose-acp}
[[ -x "$BINARY" ]] || { echo "Missing executable: $BINARY" >&2; exit 1; }
# Only system dylibs are permitted: no unshipped Homebrew or @rpath libraries.
DEPENDENCIES=$(otool -L "$BINARY" | tail -n +2 | sed -E 's/^[[:space:]]*//; s/ \(compatibility version.*$//')
while IFS= read -r dependency; do
  case "$dependency" in
    /usr/lib/*|/System/Library/*) ;;
    *) echo "Unbundled Goose dependency: $dependency" >&2; exit 1 ;;
  esac
done <<< "$DEPENDENCIES"
"$BINARY" --version
"$BINARY" --help | grep -Fq 'Usage: goose-acp'
