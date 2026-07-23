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

# Extract the host port from a postgres:// or redis:// URL. Empty when absent
# or unparsable (e.g. unix sockets / missing authority).
url_host_port() {
  local url="$1"
  local authority
  authority="${url#*://}"
  authority="${authority%%/*}"
  authority="${authority##*@}"
  if [[ "${authority}" == *:* ]]; then
    echo "${authority##*:}"
  fi
}

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

  export DATABASE_URL="${DATABASE_URL:-postgres://buzz:buzz_dev@localhost:5432/buzz}"
  export PGHOST="${PGHOST:-localhost}"
  export PGPORT="${PGPORT:-5432}"
  export PGUSER="${PGUSER:-buzz}"
  export PGPASSWORD="${PGPASSWORD:-buzz_dev}"
  export PGDATABASE="${PGDATABASE:-buzz}"
  export REDIS_URL="${REDIS_URL:-redis://localhost:6379}"

  # Host publish port for buzz-redis. Prefer an explicit override; otherwise
  # derive from REDIS_URL so a remapped URL alone is enough.
  if [[ -z "${BUZZ_REDIS_HOST_PORT:-}" ]]; then
    BUZZ_REDIS_HOST_PORT="$(url_host_port "${REDIS_URL}")"
  fi
  export BUZZ_REDIS_HOST_PORT="${BUZZ_REDIS_HOST_PORT:-6379}"
}

# Fail when host-port knobs disagree with the connection URLs Compose clients
# will actually dial. Keeps "I changed PGPORT but forgot DATABASE_URL" from
# becoming a mysterious connection refused after a healthy `docker compose up`.
validate_host_port_urls() {
  local db_port redis_port
  db_port="$(url_host_port "${DATABASE_URL}")"
  if [[ -n "${db_port}" && "${db_port}" != "${PGPORT}" ]]; then
    error "PGPORT (${PGPORT}) does not match the port in DATABASE_URL (${db_port})."
    error "Update both together, e.g. PGPORT=${db_port} and DATABASE_URL=...@localhost:${db_port}/buzz"
    exit 1
  fi

  redis_port="$(url_host_port "${REDIS_URL}")"
  if [[ -n "${redis_port}" && "${redis_port}" != "${BUZZ_REDIS_HOST_PORT}" ]]; then
    error "BUZZ_REDIS_HOST_PORT (${BUZZ_REDIS_HOST_PORT}) does not match the port in REDIS_URL (${redis_port})."
    error "Update both together, e.g. BUZZ_REDIS_HOST_PORT=${redis_port} and REDIS_URL=redis://localhost:${redis_port}"
    exit 1
  fi
}

cleanup_legacy_sprout_containers() {
  local legacy_containers
  legacy_containers=$(docker ps -a --format '{{.Names}}' | grep -E '^sprout-(postgres|redis|adminer|keycloak|minio|minio-init|prometheus)$' || true)
  if [[ -z "${legacy_containers}" ]]; then
    return
  fi

  warn "Stopping/removing legacy sprout-* dev containers so buzz-* containers can bind the standard ports"
  echo "${legacy_containers}" | xargs docker stop >/dev/null 2>&1 || true
  echo "${legacy_containers}" | xargs docker rm >/dev/null 2>&1 || true
  success "Legacy sprout-* containers removed (volumes preserved)"
}

# List PIDs listening on a TCP port that look like a host Redis server (not
# Docker's published buzz-redis). Empty when none / lsof unavailable.
host_redis_listener_pids() {
  local port="$1"
  if ! command -v lsof >/dev/null 2>&1; then
    return
  fi
  lsof -nP -iTCP:"${port}" -sTCP:LISTEN 2>/dev/null \
    | awk 'NR > 1 && $1 == "redis-ser" {print $2}' \
    | sort -u \
    | tr '\n' ' '
}

fail_if_local_redis_blocks_compose() {
  if docker ps --format '{{.Names}}' | grep -qx 'buzz-redis'; then
    return
  fi
  local redis_pids
  redis_pids="$(host_redis_listener_pids "${BUZZ_REDIS_HOST_PORT}")"
  if [[ -n "${redis_pids}" ]]; then
    error "Local Redis is already listening on port ${BUZZ_REDIS_HOST_PORT} (pid(s): ${redis_pids})."
    error "Stop it, or remap Buzz by setting BUZZ_REDIS_HOST_PORT and REDIS_URL together (see .env.example)."
    exit 1
  fi
}

fail_if_local_postgres_blocks_compose() {
  if ! command -v lsof >/dev/null 2>&1; then
    return
  fi
  if docker ps --format '{{.Names}}' | grep -qx 'buzz-postgres'; then
    return
  fi
  local pg_pids
  # Match both "postgres" and truncated "postgre" COMMAND names from lsof.
  pg_pids=$(lsof -nP -iTCP:"${PGPORT}" -sTCP:LISTEN 2>/dev/null \
    | awk 'NR > 1 && ($1 == "postgres" || $1 == "postgre") {print $2}' \
    | sort -u \
    | tr '\n' ' ' || true)
  if [[ -n "${pg_pids}" ]]; then
    error "Local Postgres is already listening on port ${PGPORT} (pid(s): ${pg_pids})."
    error "Stop it, or remap Buzz by setting PGPORT and DATABASE_URL together (see .env.example)."
    exit 1
  fi
}

postgres_accepting_connections() {
  # Always probe the container-internal port — host PGPORT only affects publish.
  docker exec buzz-postgres \
    pg_isready -h localhost -p 5432 -U "${PGUSER}" -d "${PGDATABASE}" \
    >/dev/null 2>&1
}

load_env
validate_host_port_urls
cleanup_legacy_sprout_containers
fail_if_local_postgres_blocks_compose
fail_if_local_redis_blocks_compose

# ---- Start services ---------------------------------------------------------

log "Starting services and waiting for health..."
"${REPO_ROOT}/bin/just" _ensure-services

# ---- Run migrations ---------------------------------------------------------

log "Running database migrations..."
attempts=0
max_attempts=10
until postgres_accepting_connections; do
  attempts=$((attempts + 1))
  if [[ ${attempts} -ge ${max_attempts} ]]; then
    error "Postgres did not accept connections after ${max_attempts} attempts"
    exit 1
  fi
  log "Postgres not ready for connections yet, retrying in 2s... (${attempts}/${max_attempts})"
  sleep 2
done

"${REPO_ROOT}/bin/cargo" run -p buzz-admin -- migrate
"${REPO_ROOT}/scripts/seed-local-community.sh"
success "Database migrations complete"

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
# Install into the shared .git/hooks directory using --path-format=absolute so the
# stored hooksPath is always an absolute path. Without it, --git-common-dir returns
# ".git" from the main checkout; a relative hooksPath would silently break
# linked-worktree dispatch (same failure mode as the old worktree-relative .hooks).
HOOKS_DIR="$(git -C "${REPO_ROOT}" rev-parse --path-format=absolute --git-common-dir)/hooks"
git -C "${REPO_ROOT}" config --local core.hooksPath "$HOOKS_DIR"
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
echo -e "  ${BLUE}Adminer${NC}     http://localhost:8082  (DB browser)"
echo -e "  ${BLUE}Keycloak${NC}    http://localhost:8180  (admin / admin — local OAuth testing)"
echo ""
if [[ "${PGPORT}" != "5432" || "${BUZZ_REDIS_HOST_PORT}" != "6379" ]]; then
  echo -e "  ${YELLOW}Host ports remapped:${NC} Postgres ${PGPORT}, Redis ${BUZZ_REDIS_HOST_PORT}"
  echo ""
fi
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
