#!/bin/bash
# Run one shard of the mobile Dart tests, split by file.
#
# `flutter test --total-shards` splits tests, so every shard still compiles
# and loads every test file. Splitting files means each shard compiles only
# its own. Files are balanced by line count, largest first, so the split is
# deterministic and needs no timing history.
#
# Usage: mobile-test-shard.sh INDEX TOTAL [flutter test args...]
set -euo pipefail
index=$1
total=$2
shift 2
cd "$(dirname "$0")/../mobile"

files=()
while IFS= read -r file; do
  files+=("$file")
done < <(
  find test -name '*_test.dart' -exec wc -l {} + |
    awk '$2 != "total" { print $1, $2 }' |
    sort -k1,1nr -k2,2 |
    awk -v index_="$index" -v total="$total" '
      {
        best = 0
        for (i = 1; i < total; i++) if (load[i] < load[best]) best = i
        load[best] += $1
        if (best == index_) print $2
      }'
)

if [ ${#files[@]} -eq 0 ]; then
  echo "Shard $index of $total has no test files" >&2
  exit 1
fi
echo "Shard $index of $total: ${#files[@]} test files"
flutter test "$@" "${files[@]}"
