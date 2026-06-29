#!/usr/bin/env bash
# Seed local dev host -> community rows for row-zero host binding.
#
# The relay intentionally fails closed when the request Host header is not in
# `communities`. Local dev uses loopback hosts, so bootstrap must create those
# rows after migrations before desktop/Tauri HTTP bridge calls can succeed.
#
# Host derivation lives in seed-hosts.py and the upsert in seed-communities.sql,
# kept out of this shell script on purpose: inlining the Python as a heredoc
# inside `$(...)` made bash 3.2 (stock macOS /bin/bash) fail to parse the script
# at all. The shell now only wires the two together and never parses their
# source, so it is portable across bash versions.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

if [[ -f ".env" ]]; then
  set -o allexport
  # shellcheck disable=SC1091
  source .env
  set +o allexport
fi

export PGHOST="${PGHOST:-localhost}"
export PGPORT="${PGPORT:-5432}"
export PGUSER="${PGUSER:-buzz}"
export PGPASSWORD="${PGPASSWORD:-buzz_dev}"
export PGDATABASE="${PGDATABASE:-buzz}"
export RELAY_URL="${RELAY_URL:-ws://localhost:3000}"

hosts="$(python3 "${SCRIPT_DIR}/seed-hosts.py")"
if [[ -z "${hosts}" ]]; then
  echo "error: could not derive any community host from RELAY_URL=${RELAY_URL}" >&2
  exit 1
fi

sql_file="${SCRIPT_DIR}/seed-communities.sql"

if command -v psql >/dev/null 2>&1; then
  PGPASSWORD="${PGPASSWORD}" psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" \
    -v ON_ERROR_STOP=1 -v hosts="${hosts}" -f "${sql_file}"
elif docker exec buzz-postgres psql --version >/dev/null 2>&1; then
  docker exec -i -e PGPASSWORD="${PGPASSWORD}" buzz-postgres \
    psql -U "${PGUSER}" -d "${PGDATABASE}" -v ON_ERROR_STOP=1 -v hosts="${hosts}" < "${sql_file}"
else
  echo "error: neither psql nor buzz-postgres docker psql is available" >&2
  exit 1
fi

echo "Seeded local dev community host(s):"
while IFS= read -r host; do
  [[ -n "${host}" ]] && echo "  - ${host}"
done <<< "${hosts}"
