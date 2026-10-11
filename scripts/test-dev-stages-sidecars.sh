#!/usr/bin/env bash
# Guards the `just dev` sidecar contract.
#
# `_ensure-sidecar-stubs` touches 0-byte placeholders so Tauri's compile-time
# externalBin check passes. Any lane that then launches the app must overwrite
# those placeholders with the real binaries, or the app ships files it cannot
# execute and agent sign-in fails as "Sign-in unavailable" (#4747, #3927).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
JUSTFILE="${REPO_ROOT}/Justfile"

# Body of a just recipe: from its header line until the next top-level line.
recipe_body() {
  awk -v name="$1" '
    $0 ~ "^" name "( |:)" { inside = 1; next }
    inside && /^[^ \t]/ && !/^$/ { exit }
    inside { print }
  ' "${JUSTFILE}"
}

# Sidecars stubbed by _ensure-sidecar-stubs, including the non-Windows extras.
stubbed="$(recipe_body '_ensure-sidecar-stubs' \
  | sed -n 's/.*SIDECARS[+]*=(\([^)]*\)).*/\1/p' \
  | tr ' ' '\n' | sed '/^$/d' | sort -u)"

if [[ -z "${stubbed}" ]]; then
  echo "FAIL: could not parse the sidecar list from _ensure-sidecar-stubs" >&2
  exit 1
fi

status=0

# Every lane that boots the desktop app from a debug build must stage real binaries.
for lane in dev desktop-standalone; do
  body="$(recipe_body "${lane}")"

  if ! grep -q 'cp .*desktop/src-tauri/binaries/' <<<"${body}"; then
    echo "FAIL: \`just ${lane}\` never copies real sidecars over the 0-byte stubs;" >&2
    echo "      the app would launch with placeholders it cannot execute." >&2
    status=1
    continue
  fi

  staged="$(sed -n 's/.*for bin in \([^;]*\);.*/\1/p' <<<"${body}" \
    | tr ' ' '\n' | sed '/^$/d' | sort -u)"

  if [[ "${staged}" != "${stubbed}" ]]; then
    echo "FAIL: \`just ${lane}\` stages a different sidecar set than _ensure-sidecar-stubs." >&2
    echo "      stubbed: $(tr '\n' ' ' <<<"${stubbed}")" >&2
    echo "      staged:  $(tr '\n' ' ' <<<"${staged}")" >&2
    echo "      A stubbed-but-unstaged sidecar ships as an unexecutable 0-byte file." >&2
    status=1
  fi
done

if [[ "${status}" -eq 0 ]]; then
  echo "PASS: dev and desktop-standalone stage every stubbed sidecar"
fi

exit "${status}"
