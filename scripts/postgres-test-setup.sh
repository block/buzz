#!/usr/bin/env bash
# Build one desired-state source database for a nextest PostgreSQL run. Each
# selected test clones it (or template0 for migration-owned tests). The outer
# postgres-test-run.sh process owns and removes the source database.
set -euo pipefail

: "${NEXTEST_ENV:?nextest must provide NEXTEST_ENV}"
: "${BUZZ_POSTGRES_ADMIN_URL:?set BUZZ_POSTGRES_ADMIN_URL to an administrator database URL}"
: "${BUZZ_POSTGRES_DESIRED_TEMPLATE:?postgres-test-run.sh must provide the desired-state database name}"
: "${PGHOST:?set PGHOST for pgschema}"
: "${PGPORT:?set PGPORT for pgschema}"
: "${PGUSER:?set PGUSER for pgschema}"
: "${PGPASSWORD:?set PGPASSWORD for pgschema}"

resolve_pg_command() {
  local name="$1"
  local candidate
  if [[ -n "${PG_BIN_DIR:-}" ]]; then
    candidate="${PG_BIN_DIR}/${name}"
  else
    candidate="$(command -v "$name" || true)"
  fi
  if [[ -z "$candidate" || ! -x "$candidate" ]]; then
    echo "required PostgreSQL client is not executable: ${candidate:-$name}" >&2
    exit 1
  fi
  printf '%s\n' "$candidate"
}

psql="$(resolve_pg_command psql)"
createdb="$(resolve_pg_command createdb)"
dropdb="$(resolve_pg_command dropdb)"

workspace_root="${NEXTEST_WORKSPACE_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
template_database="$BUZZ_POSTGRES_DESIRED_TEMPLATE"
if [[ ! "$template_database" =~ ^buzz_nt_[0-9a-f]{20}_desired$ ]]; then
  echo "refusing unsafe desired-state database name: $template_database" >&2
  exit 1
fi
run_hash="${template_database#buzz_nt_}"
run_hash="${run_hash%_desired}"

"$dropdb" --if-exists --force \
  --maintenance-db="$BUZZ_POSTGRES_ADMIN_URL" \
  "$template_database" >/dev/null 2>&1
"$createdb" \
  --maintenance-db="$BUZZ_POSTGRES_ADMIN_URL" \
  --template=template0 \
  "$template_database"

export PGDATABASE="$template_database"
export PGSCHEMA_PLAN_HOST="${PGSCHEMA_PLAN_HOST:-$PGHOST}"
export PGSCHEMA_PLAN_PORT="${PGSCHEMA_PLAN_PORT:-$PGPORT}"
export PGSCHEMA_PLAN_DB="$template_database"
export PGSCHEMA_PLAN_USER="${PGSCHEMA_PLAN_USER:-$PGUSER}"
export PGSCHEMA_PLAN_PASSWORD="${PGSCHEMA_PLAN_PASSWORD:-$PGPASSWORD}"

schema_log="${TMPDIR:-/tmp}/buzz-pgschema-${run_hash}.log"
if ! "$workspace_root/bin/pgschema" apply \
  --file "$workspace_root/schema/schema.sql" \
  --auto-approve >"$schema_log" 2>&1; then
  cat "$schema_log" >&2
  exit 1
fi
if ! "$psql" --dbname="$template_database" --set=ON_ERROR_STOP=1 \
  --file="$workspace_root/scripts/reconcile-schema-after-pgschema.sql" \
  >>"$schema_log" 2>&1; then
  cat "$schema_log" >&2
  exit 1
fi
rm -f "$schema_log"

printf 'BUZZ_POSTGRES_DESIRED_TEMPLATE=%s\n' "$template_database" >>"$NEXTEST_ENV"
