#!/usr/bin/env bash
set -euo pipefail

ENV_FILE="${1:-.env}"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "error: ${ENV_FILE} does not exist" >&2
  exit 1
fi

existing_key="$({
  unset BUZZ_RELAY_PRIVATE_KEY
  set +u
  # shellcheck disable=SC1090
  source "${ENV_FILE}" || exit 1
  printf '%s' "${BUZZ_RELAY_PRIVATE_KEY:-}"
})"

if [[ -n "${existing_key}" ]]; then
  chmod 600 "${ENV_FILE}"
  exit 0
fi

relay_key="$(node <<'NODE'
const { randomBytes } = require("node:crypto");
const curveOrder = BigInt(
  "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141",
);

let bytes;
let scalar;
do {
  bytes = randomBytes(32);
  scalar = BigInt(`0x${bytes.toString("hex")}`);
} while (scalar === 0n || scalar >= curveOrder);

process.stdout.write(bytes.toString("hex"));
NODE
)"

temp_file="$(mktemp "${ENV_FILE}.tmp.XXXXXX")"
trap 'rm -f "${temp_file}"' EXIT

awk -v key="${relay_key}" '
  BEGIN { replaced = 0 }
  /^[[:space:]]*(export[[:space:]]+)?BUZZ_RELAY_PRIVATE_KEY=/ {
    if (!replaced) {
      print "BUZZ_RELAY_PRIVATE_KEY=" key
      replaced = 1
    }
    next
  }
  { print }
  END {
    if (!replaced) {
      if (NR > 0) print ""
      print "BUZZ_RELAY_PRIVATE_KEY=" key
    }
  }
' "${ENV_FILE}" > "${temp_file}"

chmod 600 "${temp_file}"
mv "${temp_file}" "${ENV_FILE}"
trap - EXIT

echo "Generated BUZZ_RELAY_PRIVATE_KEY in ${ENV_FILE}."
