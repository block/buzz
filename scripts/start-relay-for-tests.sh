#!/usr/bin/env bash
# =============================================================================
# start-relay-for-tests.sh — Start the Buzz relay and its backing services
# =============================================================================
# Shared script for CI jobs that need a running relay. Starts docker compose
# services, waits for health, applies the schema, builds the relay, starts it,
# and polls readiness.
#
# Usage:
#   ./scripts/start-relay-for-tests.sh [--profile <cargo-profile>] [--no-build]
#
# Options:
#   --profile <profile>   Cargo build profile (default: ci)
#   --no-build            Use existing target/<profile>/ binaries (CI artifact reuse)
#
# Exports:
#   RELAY_URL=ws://localhost:3000
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ── Defaults ──────────────────────────────────────────────────────────────────

CARGO_PROFILE="${CARGO_PROFILE:-ci}"
SKIP_BUILD=false

# ── Parse args ────────────────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      CARGO_PROFILE="$2"
      shift 2
      ;;
    --no-build)
      SKIP_BUILD=true
      shift
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

# ── Colors ────────────────────────────────────────────────────────────────────

BLUE='\033[0;34m'
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

log()   { echo -e "${BLUE}[relay-test]${NC} $*"; }
ok()    { echo -e "${GREEN}[relay-test]${NC} $*"; }
err()   { echo -e "${RED}[relay-test]${NC} $*" >&2; }

# ── Start docker compose services ────────────────────────────────────────────

cd "${REPO_ROOT}"

log "Starting docker compose services..."
docker compose up -d postgres redis minio minio-init

# ── Wait for services to be healthy ──────────────────────────────────────────

wait_healthy() {
  local service="$1"
  local container="$2"
  log "Waiting for ${service}..."
  for attempt in $(seq 1 60); do
    status=$(docker inspect --format='{{.State.Health.Status}}' "${container}" 2>/dev/null || echo "not_found")
    if [ "${status}" = "healthy" ]; then
      ok "${service} is healthy"
      return 0
    fi
    sleep 2
  done
  err "${service} did not become healthy within 120s"
  docker logs "${container}" || true
  return 1
}

wait_healthy "Postgres" "buzz-postgres"
wait_healthy "Redis" "buzz-redis"
wait_healthy "MinIO" "buzz-minio"

# ── Apply database schema ────────────────────────────────────────────────────

log "Applying database schema..."
export PGHOST=localhost
export PGPORT=5432
export PGUSER=buzz
export PGPASSWORD=buzz_dev
export PGDATABASE=buzz

# Use the already-running docker postgres for desired-state planning instead of
# downloading an embedded Postgres from Maven Central (transient-fetch flake source).
export PGSCHEMA_PLAN_HOST=localhost
export PGSCHEMA_PLAN_PORT=5432
export PGSCHEMA_PLAN_DB=buzz
export PGSCHEMA_PLAN_USER=buzz
export PGSCHEMA_PLAN_PASSWORD=buzz_dev

./bin/pgschema apply --file schema/schema.sql --auto-approve
docker exec -i -e PGPASSWORD="${PGPASSWORD}" buzz-postgres \
  psql -U "${PGUSER}" -d "${PGDATABASE}" -v ON_ERROR_STOP=1 < scripts/attach-schema-partitions.sql
ok "Schema applied"

# ── Seed the deployment community ────────────────────────────────────────────
# Multi-tenant: the relay resolves every connection's tenant from the durable
# communities host map (WHERE host = normalize_host($1)). normalize_host keeps
# non-default ports, so the host must be 'localhost:3000' verbatim to match
# RELAY_URL=ws://localhost:3000. The relay never auto-seeds a community
# (ensure_configured_community has no callers) and fails closed on an unmapped
# host, so without this row every e2e connection would 404 at host-binding.
# The unique index is on lower(host), so ON CONFLICT must target that expression.
# psql is not on PATH in the hermit env; postgres runs as the buzz-postgres
# docker container, so exec into it (same fallback as setup-desktop-test-data.sh).
log "Seeding deployment community (host=localhost:3000)..."
if command -v psql >/dev/null 2>&1; then
  seed_psql() { PGPASSWORD="${PGPASSWORD}" psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" -qtA "$@"; }
else
  seed_psql() { docker exec -e PGPASSWORD="${PGPASSWORD}" buzz-postgres psql -U "${PGUSER}" -d "${PGDATABASE}" -qtA "$@"; }
fi
seed_psql -c "
INSERT INTO communities (id, host)
VALUES ('00000000-0000-4000-8000-00000000c0de', 'localhost:3000')
ON CONFLICT (lower(host)) DO NOTHING
;
"
ok "Community seeded"

# ── Build relay ──────────────────────────────────────────────────────────────

if [[ "${SKIP_BUILD}" == "true" ]]; then
  for bin in buzz-relay git-credential-nostr; do
    if [[ ! -x "./target/${CARGO_PROFILE}/${bin}" ]]; then
      err "--no-build: ./target/${CARGO_PROFILE}/${bin} missing or not executable"
      exit 1
    fi
  done
  log "Skipping relay build (--no-build); using existing target/${CARGO_PROFILE}/ binaries"
else
  log "Building relay (profile: ${CARGO_PROFILE})..."
  cargo build --profile "${CARGO_PROFILE}" -p buzz-relay -p git-credential-nostr
  ok "Relay built"
fi

# ── Start relay ──────────────────────────────────────────────────────────────

log "Starting relay..."
nohup env \
  DATABASE_URL=postgres://buzz:buzz_dev@localhost:5432/buzz \
  REDIS_URL=redis://localhost:6379 \
  RELAY_URL=ws://localhost:3000 \
  BUZZ_BIND_ADDR=0.0.0.0:3000 \
  BUZZ_REQUIRE_AUTH_TOKEN=false \
  BUZZ_RECONCILE_CHANNELS=true \
  BUZZ_GIT_PROBE_WRITERS=8 \
  "./target/${CARGO_PROFILE}/buzz-relay" > /tmp/buzz-relay.log 2>&1 &
echo $! > /tmp/buzz-relay.pid

# ── Poll readiness ───────────────────────────────────────────────────────────

log "Waiting for relay readiness..."
for attempt in $(seq 1 60); do
  if ! kill -0 "$(cat /tmp/buzz-relay.pid)" 2>/dev/null; then
    err "Relay process died"
    cat /tmp/buzz-relay.log
    exit 1
  fi
  status_code=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:3000/_readiness || true)
  if [ "${status_code}" = "200" ]; then
    ok "Relay is ready at ws://localhost:3000"
    export RELAY_URL=ws://localhost:3000
    exit 0
  fi
  sleep 1
done

err "Relay did not become ready within 60s"
cat /tmp/buzz-relay.log
exit 1
