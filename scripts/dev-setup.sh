#!/usr/bin/env bash
# =============================================================================
# dev-setup.sh — One-shot local dev environment setup
# =============================================================================
# Usage: ./scripts/dev-setup.sh
#
# Starts Docker services, waits for healthy, runs migrations, installs desktop
# deps, and prints next steps.
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log()     { echo -e "${BLUE}[dev-setup]${NC} $*"; }
success() { echo -e "${GREEN}[dev-setup]${NC} $*"; }
warn()    { echo -e "${YELLOW}[dev-setup]${NC} $*"; }
error()   { echo -e "${RED}[dev-setup]${NC} $*" >&2; }

# ---- Preflight checks -------------------------------------------------------

if ! command -v docker &>/dev/null; then
  error "Docker not found. Install Docker Desktop: https://www.docker.com/products/docker-desktop/"
  exit 1
fi

if ! docker info &>/dev/null; then
  error "Docker daemon is not running. Start Docker Desktop and try again."
  exit 1
fi

cd "${REPO_ROOT}"

# ---- Load environment -------------------------------------------------------

load_env() {
  if [[ -f ".env" ]]; then
    log "Loading .env..."
    set -o allexport
    # shellcheck disable=SC1091
    source .env
    set +o allexport
  fi

  # Smooth the local rename path for developers with a pre-Buzz .env copied
  # from .env.example. Only rewrite the old default values; custom values stay
  # untouched.
  if [[ "${DATABASE_URL:-}" == "postgres://sprout:sprout_dev@localhost:5432/sprout" ]]; then
    warn "Migrating legacy default DATABASE_URL from sprout to buzz for this setup run"
    DATABASE_URL="postgres://buzz:buzz_dev@localhost:5432/buzz"
  fi
  if [[ "${PGUSER:-}" == "sprout" ]]; then PGUSER="buzz"; fi
  if [[ "${PGPASSWORD:-}" == "sprout_dev" ]]; then PGPASSWORD="buzz_dev"; fi
  if [[ "${PGDATABASE:-}" == "sprout" ]]; then PGDATABASE="buzz"; fi
  if [[ "${TYPESENSE_API_KEY:-}" == "sprout_dev_key" ]]; then TYPESENSE_API_KEY="buzz_dev_key"; fi

  export DATABASE_URL="${DATABASE_URL:-postgres://buzz:buzz_dev@localhost:5432/buzz}"
  export PGHOST="${PGHOST:-localhost}"
  export PGPORT="${PGPORT:-5432}"
  export PGUSER="${PGUSER:-buzz}"
  export PGPASSWORD="${PGPASSWORD:-buzz_dev}"
  export PGDATABASE="${PGDATABASE:-buzz}"
  export REDIS_URL="${REDIS_URL:-redis://localhost:6379}"
  export TYPESENSE_API_KEY="${TYPESENSE_API_KEY:-buzz_dev_key}"
  export TYPESENSE_URL="${TYPESENSE_URL:-http://localhost:8108}"
}

cleanup_legacy_sprout_containers() {
  local legacy_containers
  legacy_containers=$(docker ps -a --format '{{.Names}}' | grep -E '^sprout-(postgres|redis|typesense|adminer|keycloak|minio|minio-init|prometheus)$' || true)
  if [[ -z "${legacy_containers}" ]]; then
    return
  fi

  warn "Stopping/removing legacy sprout-* dev containers so buzz-* containers can bind the standard ports"
  echo "${legacy_containers}" | xargs docker stop >/dev/null 2>&1 || true
  echo "${legacy_containers}" | xargs docker rm >/dev/null 2>&1 || true
  success "Legacy sprout-* containers removed (volumes preserved)"
}

fail_if_local_redis_blocks_compose() {
  if ! command -v lsof >/dev/null 2>&1; then
    return
  fi
  if docker ps --format '{{.Names}}' | grep -qx 'buzz-redis'; then
    return
  fi
  local redis_pids
  redis_pids=$(lsof -nP -iTCP:6379 -sTCP:LISTEN 2>/dev/null | awk 'NR > 1 && $1 == "redis-ser" {print $2}' | sort -u | tr '
' ' ' || true)
  if [[ -n "${redis_pids}" ]]; then
    error "Local Redis is already listening on port 6379 (pid(s): ${redis_pids}). Stop it before running setup: brew services stop redis"
    exit 1
  fi
}

postgres_accepting_connections() {
  docker exec buzz-postgres \
    pg_isready -h localhost -p 5432 -U "${PGUSER}" -d "${PGDATABASE}" \
    >/dev/null 2>&1
}

load_env
cleanup_legacy_sprout_containers
fail_if_local_redis_blocks_compose

# ---- Start services ---------------------------------------------------------

log "Starting services and waiting for health..."
"${REPO_ROOT}/bin/just" _ensure-services

# ---- Run migrations ---------------------------------------------------------

log "Running database migrations..."

PGSCHEMA="${REPO_ROOT}/bin/pgschema"
SCHEMA_FILE="${REPO_ROOT}/schema/schema.sql"

if [[ ! -f "${SCHEMA_FILE}" ]]; then
  warn "No schema.sql found at ${SCHEMA_FILE}. Skipping."
else
  if [[ -x "${PGSCHEMA}" ]]; then
    # pgschema uses CREATE INDEX CONCURRENTLY for new indexes, which Postgres
    # does not support on partitioned tables. Pre-create any such indexes here
    # so pgschema sees them as already existing and skips the CONCURRENTLY path.
    log "Pre-creating indexes on partitioned tables (if needed)..."
    docker exec buzz-postgres psql -U "${PGUSER}" -d "${PGDATABASE}" -q -c \
      "CREATE INDEX IF NOT EXISTS idx_events_parameterized ON events (kind, pubkey, d_tag, deleted_at) WHERE d_tag IS NOT NULL;" \
      2>/dev/null || true

    log "Using pgschema for migrations..."
    attempts=0
    max_attempts=10
    pgschema_output="$(mktemp)"
    trap 'rm -f "${pgschema_output}"' EXIT
    until "${PGSCHEMA}" apply --file "${SCHEMA_FILE}" --auto-approve >"${pgschema_output}" 2>&1; do
      attempts=$((attempts + 1))
      if postgres_accepting_connections; then
        error "pgschema failed even though Postgres is accepting connections"
        cat "${pgschema_output}" >&2
        exit 1
      fi
      if [[ ${attempts} -ge ${max_attempts} ]]; then
        error "Failed to run migrations after ${max_attempts} attempts"
        cat "${pgschema_output}" >&2
        exit 1
      fi
      log "Postgres not ready for connections yet, retrying in 2s... (${attempts}/${max_attempts})"
      sleep 2
    done
    success "Migrations applied via pgschema"

    # Run data backfills (idempotent — safe to re-run).
    BACKFILL_DIR="${REPO_ROOT}/scripts"
    if [[ -f "${BACKFILL_DIR}/backfill-d-tag.sql" ]]; then
      log "Running d_tag backfill for NIP-33 events..."
      if psql "${DATABASE_URL}" -f "${BACKFILL_DIR}/backfill-d-tag.sql" 2>/dev/null; then
        success "d_tag backfill complete"
      else
        warn "d_tag backfill failed (relay startup will retry automatically)"
      fi
    fi
  else
    error "pgschema not found at ${PGSCHEMA}. Run: ./bin/hermit install pgschema"
    exit 1
  fi
fi

# ---- Install desktop dependencies -------------------------------------------

DESKTOP_DIR="${REPO_ROOT}/desktop"

if [[ -d "${DESKTOP_DIR}" ]]; then
  if command -v pnpm &>/dev/null; then
    log "Installing desktop dependencies (pnpm install)..."
    (cd "${DESKTOP_DIR}" && pnpm install)
    success "Desktop dependencies installed"
  else
    warn "pnpm not found — skipping desktop dependency install."
    warn "Run '. ./bin/activate-hermit' to get pnpm, then 'just desktop-install'."
  fi
else
  warn "Desktop directory not found at ${DESKTOP_DIR} — skipping."
fi

# ---- Install web dependencies -----------------------------------------------

WEB_DIR="${REPO_ROOT}/web"

if [[ -d "${WEB_DIR}" ]]; then
  if command -v pnpm &>/dev/null; then
    log "Installing web dependencies (pnpm install)..."
    (cd "${WEB_DIR}" && pnpm install)
    success "Web dependencies installed"
  else
    warn "pnpm not found — skipping web dependency install."
    warn "Run '. ./bin/activate-hermit' to get pnpm, then 'just desktop-install'."
  fi
else
  warn "Web directory not found at ${WEB_DIR} — skipping."
fi

# ---- Install git hooks ------------------------------------------------------

log "Installing git hooks..."
git config --local core.hooksPath .hooks
lefthook install --force
success "Git hooks installed"

# ---- Print connection info --------------------------------------------------

echo ""
echo -e "${GREEN}=======================================================${NC}"
echo -e "${GREEN}  Buzz dev environment is ready!${NC}"
echo -e "${GREEN}=======================================================${NC}"
echo ""
echo -e "  ${BLUE}Postgres${NC}    ${DATABASE_URL}"
echo -e "  ${BLUE}Redis${NC}       ${REDIS_URL}"
echo -e "  ${BLUE}Typesense${NC}   ${TYPESENSE_URL}  (key: ${TYPESENSE_API_KEY})"
echo -e "  ${BLUE}Adminer${NC}     http://localhost:8082  (DB browser)"
echo -e "  ${BLUE}Keycloak${NC}    http://localhost:8180  (admin / admin — local OAuth testing)"
echo ""
echo -e "  ${YELLOW}Next steps:${NC}"
echo -e "    just relay                              # start the relay (terminal 1)"
echo -e "    just dev                                # start the desktop app (terminal 2)"
echo ""
echo -e "  ${YELLOW}Useful commands:${NC}"
echo -e "    docker compose ps             # check service status"
echo -e "    docker compose logs -f        # tail all logs"
echo -e "    docker compose down           # stop services (keep data)"
echo -e "    ./scripts/dev-reset.sh        # wipe and start fresh"
echo ""

exit 0
