#!/usr/bin/env bash
# Run deterministic desktop correctness smoke; timing metrics are informational.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="${BUZZ_RELEASE_SMOKE_ARTIFACT_DIR:-${ROOT}/desktop/test-results/release-smoke}"
DB_NAME="${BUZZ_RELEASE_SMOKE_DB:-buzz_release_smoke_${$}}"
REDIS_DB="${BUZZ_RELEASE_SMOKE_REDIS_DB:-}"
LOCK_DIR="${TMPDIR:-/tmp}/buzz-desktop-release-smoke.lock"
RELAY_PID=""
LOCK_HELD=false

free_port() {
  python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

RELAY_PORT="${BUZZ_RELEASE_SMOKE_RELAY_PORT:-$(free_port)}"
HEALTH_PORT="${BUZZ_RELEASE_SMOKE_HEALTH_PORT:-$(free_port)}"
METRICS_PORT="${BUZZ_RELEASE_SMOKE_METRICS_PORT:-$(free_port)}"
COMMUNITY_HOST="localhost:${RELAY_PORT}"
RELAY_HTTP_URL="http://${COMMUNITY_HOST}"
STARTED_AT="$(date +%s)"

log() { printf '[desktop-release-smoke] %s\n' "$*"; }
phase() {
  local name="$1" start="$2"
  printf '{"phase":"%s","duration_ms":%d}\n' "$name" "$(( ($(date +%s) - start) * 1000 ))" >> "${ARTIFACT_DIR}/phases.jsonl"
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ -n "${RELAY_PID}" ]]; then
    kill "${RELAY_PID}" 2>/dev/null || true
    for _ in $(seq 1 50); do
      kill -0 "${RELAY_PID}" 2>/dev/null || break
      sleep 0.1
    done
    kill -9 "${RELAY_PID}" 2>/dev/null || true
  fi
  docker exec buzz-redis redis-cli -n "${REDIS_DB}" FLUSHDB >/dev/null 2>&1 || true
  docker exec buzz-postgres dropdb -U buzz --if-exists "${DB_NAME}" >/dev/null 2>&1 || true
  if [[ "${LOCK_HELD}" == true ]]; then rmdir "${LOCK_DIR}" 2>/dev/null || true; fi
  exit "${status}"
}
trap cleanup EXIT INT TERM

# Shared Docker services expose one Redis instance and a finite database index
# space. Serialize automatic allocation; callers that deliberately own an
# isolated Redis DB may opt out by setting BUZZ_RELEASE_SMOKE_REDIS_DB.
if [[ -z "${REDIS_DB}" ]]; then
  mkdir "${LOCK_DIR}" 2>/dev/null || {
    log "another release-smoke run owns ${LOCK_DIR}; set BUZZ_RELEASE_SMOKE_REDIS_DB only for an isolated runner"
    exit 1
  }
  LOCK_HELD=true
  REDIS_DB=15
fi

mkdir -p "${ARTIFACT_DIR}"
: > "${ARTIFACT_DIR}/phases.jsonl"
cd "${ROOT}"

phase_start="$(date +%s)"
log "starting backing services"
docker compose up -d postgres redis minio minio-init
for container in buzz-postgres buzz-redis buzz-minio; do
  for _ in $(seq 1 60); do
    [[ "$(docker inspect --format='{{.State.Health.Status}}' "${container}" 2>/dev/null || true)" == "healthy" ]] && break
    sleep 1
  done
  [[ "$(docker inspect --format='{{.State.Health.Status}}' "${container}" 2>/dev/null || true)" == "healthy" ]] || {
    docker logs "${container}" || true
    exit 1
  }
done
phase services "${phase_start}"

phase_start="$(date +%s)"
log "creating isolated database ${DB_NAME}"
docker exec buzz-postgres createdb -U buzz "${DB_NAME}"
export PGHOST=localhost PGPORT=5432 PGUSER=buzz PGPASSWORD=buzz_dev PGDATABASE="${DB_NAME}"
export PGSCHEMA_PLAN_HOST=localhost PGSCHEMA_PLAN_PORT=5432 PGSCHEMA_PLAN_DB="${DB_NAME}"
export PGSCHEMA_PLAN_USER=buzz PGSCHEMA_PLAN_PASSWORD=buzz_dev
./bin/pgschema apply --file schema/schema.sql --auto-approve
docker exec -i -e PGPASSWORD=buzz_dev buzz-postgres \
  psql -U buzz -d "${DB_NAME}" -v ON_ERROR_STOP=1 < scripts/reconcile-schema-after-pgschema.sql
BUZZ_DB_NAME="${DB_NAME}" BUZZ_COMMUNITY_HOST="${COMMUNITY_HOST}" ./scripts/setup-desktop-test-data.sh
docker exec buzz-redis redis-cli -n "${REDIS_DB}" FLUSHDB >/dev/null
phase database "${phase_start}"

phase_start="$(date +%s)"
if [[ -n "${BUZZ_E2E_RELAY_BIN:-}" ]]; then
  RELAY_BIN="${BUZZ_E2E_RELAY_BIN}"
else
  log "building relay"
  cargo build --profile ci -p buzz-relay
  RELAY_BIN="${ROOT}/target/ci/buzz-relay"
fi
log "starting relay at ${RELAY_HTTP_URL}"
RELAY_PRIVATE_KEY="$(openssl rand -hex 32)"
env \
  DATABASE_URL="postgres://buzz:buzz_dev@localhost:5432/${DB_NAME}" \
  REDIS_URL="redis://localhost:6379/${REDIS_DB}" \
  RELAY_URL="ws://${COMMUNITY_HOST}" \
  BUZZ_BIND_ADDR="127.0.0.1:${RELAY_PORT}" \
  BUZZ_HEALTH_PORT="${HEALTH_PORT}" \
  BUZZ_METRICS_PORT="${METRICS_PORT}" \
  BUZZ_RELAY_PRIVATE_KEY="${RELAY_PRIVATE_KEY}" \
  BUZZ_REQUIRE_AUTH_TOKEN=false \
  BUZZ_RECONCILE_CHANNELS=true \
  BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN=1000000 \
  BUZZ_RATE_LIMIT_HUMAN_API_CALLS_PER_MIN=1000000 \
  BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC=100000 \
  "${RELAY_BIN}" > "${ARTIFACT_DIR}/relay.log" 2>&1 &
RELAY_PID=$!
ready=false
for _ in $(seq 1 300); do
  kill -0 "${RELAY_PID}" 2>/dev/null || { cat "${ARTIFACT_DIR}/relay.log"; exit 1; }
  if curl --silent --fail --max-time 1 "http://127.0.0.1:${HEALTH_PORT}/_readiness" >/dev/null; then
    ready=true
    break
  fi
  sleep 0.1
done
[[ "${ready}" == true ]] || { cat "${ARTIFACT_DIR}/relay.log"; exit 1; }
phase relay "${phase_start}"

phase_start="$(date +%s)"
if [[ "${BUZZ_RELEASE_SMOKE_NO_BUILD:-0}" == "1" ]]; then
  log "reusing existing desktop E2E bundle"
else
  log "building desktop E2E bundle"
  pnpm -C desktop build:e2e
fi
phase build "${phase_start}"

phase_start="$(date +%s)"
log "running release smoke"
BUZZ_E2E_RELAY_URL="${RELAY_HTTP_URL}" \
BUZZ_RELEASE_SMOKE_ARTIFACT_DIR="${ARTIFACT_DIR}" \
pnpm -C desktop exec playwright test --config=playwright.release-smoke.config.ts
phase smoke "${phase_start}"
phase total "${STARTED_AT}"
log "artifacts: ${ARTIFACT_DIR}"
