#!/usr/bin/env bash
# =============================================================================
# check-schema-drift.sh — Assert schema/schema.sql matches migrations/
# =============================================================================
# The declarative snapshot (schema/schema.sql, applied by CI and local test
# infra via `pgschema apply`) is hand-maintained and can silently drift from
# migrations/*.sql — the source of truth the relay applies to real databases
# (sqlx migrate! at startup). Drift means e2e runs against a different schema
# than production, and failures surface as opaque downstream errors instead of
# "the snapshot is out of date" (#1322).
#
# This gate makes drift unmergeable:
#   1. Start an ephemeral Postgres container (same image as docker-compose).
#   2. Apply migrations/*.sql in order, each in one transaction (matching
#      sqlx's per-migration transaction semantics).
#   3. `pgschema plan` the snapshot against that database. An empty plan means
#      the snapshot describes exactly the schema the migrations build; any
#      diff is printed and fails the check.
#
# Runs standalone: it never touches the buzz-postgres compose service or its
# data, and cleans up its own container on exit.
#
# Usage:
#   ./scripts/check-schema-drift.sh
#
# Environment:
#   DRIFT_PG_PORT   Host port for the ephemeral Postgres (default: 55432 —
#                   deliberately not 5432, which is often taken by a local
#                   Postgres that would shadow the container).
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

DRIFT_PG_PORT="${DRIFT_PG_PORT:-55432}"
PG_IMAGE="postgres:17-alpine"
CONTAINER="buzz-schema-drift-pg"
PGSCHEMA="${REPO_ROOT}/bin/pgschema"

log() { printf '\033[0;34m[drift]\033[0m %s\n' "$*"; }
ok()  { printf '\033[0;32m[drift]\033[0m %s\n' "$*"; }
err() { printf '\033[0;31m[drift]\033[0m %s\n' "$*" >&2; }

if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
  err "docker is required (starts an ephemeral Postgres); is the daemon running?"
  exit 1
fi
if [[ ! -x "${PGSCHEMA}" ]]; then
  err "bin/pgschema not found — activate hermit first (. ./bin/activate-hermit)"
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  err "jq is required to inspect the pgschema plan output"
  exit 1
fi

cleanup() { docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup

log "Starting ephemeral Postgres (${PG_IMAGE}) on port ${DRIFT_PG_PORT}..."
docker run --rm -d --name "${CONTAINER}" \
  -e POSTGRES_USER=buzz -e POSTGRES_PASSWORD=buzz_dev \
  -p "${DRIFT_PG_PORT}:5432" "${PG_IMAGE}" >/dev/null

for _ in $(seq 1 60); do
  if docker exec "${CONTAINER}" pg_isready -U buzz >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
docker exec "${CONTAINER}" pg_isready -U buzz >/dev/null 2>&1 || {
  err "Postgres did not become ready within 60s"
  exit 1
}

# drift_migrations receives migrations/*.sql; drift_plan is pgschema's scratch
# database for desired-state planning (same pattern as CI's PGSCHEMA_PLAN_*,
# avoiding the embedded-Postgres Maven download flake).
docker exec "${CONTAINER}" psql -U buzz -d postgres \
  -qc "CREATE DATABASE drift_migrations" \
  -c  "CREATE DATABASE drift_plan" >/dev/null

log "Applying $(ls migrations/*.sql | wc -l | tr -d ' ') migrations..."
for migration in migrations/*.sql; do
  # -1: one transaction per file, matching sqlx migrate! semantics (0007's
  # LOCK TABLE requires a transaction block).
  if ! docker exec -i "${CONTAINER}" psql -U buzz -d drift_migrations \
      -v ON_ERROR_STOP=1 -q -1 < "${migration}"; then
    err "migration failed to apply: ${migration}"
    exit 1
  fi
done
ok "Migrations applied"

log "Planning schema/schema.sql against the migration-built database..."
PLAN_JSON="$(mktemp)"
PLAN_HUMAN="$(mktemp)"
trap 'cleanup; rm -f "${PLAN_JSON}" "${PLAN_HUMAN}"' EXIT

PGPASSWORD=buzz_dev \
PGSCHEMA_PLAN_HOST=localhost \
PGSCHEMA_PLAN_PORT="${DRIFT_PG_PORT}" \
PGSCHEMA_PLAN_DB=drift_plan \
PGSCHEMA_PLAN_USER=buzz \
PGSCHEMA_PLAN_PASSWORD=buzz_dev \
"${PGSCHEMA}" plan \
  --host localhost --port "${DRIFT_PG_PORT}" --user buzz \
  --db drift_migrations --file schema/schema.sql \
  --no-color \
  --output-human "${PLAN_HUMAN}" \
  --output-json "${PLAN_JSON}" >/dev/null

# An empty plan serializes as `"groups": null`.
if jq -e '.groups == null or .groups == []' "${PLAN_JSON}" >/dev/null; then
  ok "schema/schema.sql matches migrations/ — no drift"
  exit 0
fi

err "schema/schema.sql does NOT match the schema built by migrations/*.sql."
err "The plan below is what pgschema would change on the migration-built"
err "database to reach the snapshot — i.e. the drift. Update schema/schema.sql"
err "(and, for new events-parent row triggers, scripts/attach-schema-partitions.sql)"
err "so this plan is empty."
echo
cat "${PLAN_HUMAN}"
exit 1
