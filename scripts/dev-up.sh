#!/usr/bin/env bash
# =============================================================================
# dev-up.sh — Provision the local dev environment on first run, start it always
# =============================================================================
# Usage: ./scripts/dev-up.sh [--relay|--web|--setup-only] [--no-build] [--yes]
#
# Puts the Hermit-pinned toolchain on PATH, starts Docker Desktop when its
# daemon is down, clears the port conflicts that make `just setup` and
# `just dev` fail, provisions a fresh checkout through `just setup`, then starts
# the relay and the desktop app. Every step is idempotent; re-running is cheap.
# =============================================================================
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log()     { echo -e "${BLUE}[dev-up]${NC} $*"; }
success() { echo -e "${GREEN}[dev-up]${NC} $*"; }
warn()    { echo -e "${YELLOW}[dev-up]${NC} $*"; }
error()   { echo -e "${RED}[dev-up]${NC} $*" >&2; }

MODE="dev"
ASSUME_YES=false
RUN_BUILD=true
INVOCATION="scripts/dev-up.sh"
RELAY_PGID=""
RUNTIME_LABEL=""
RUNTIME_APP=""

DOCKER_WAIT_SECS=120
RELAY_READY_WAIT_SECS=600
PORT_FREE_WAIT_SECS=15

# /target is gitignored, so the stamp stays out of `git status`. It is only a
# fast path: losing it to `cargo clean` costs one extra database probe.
STAMP_FILE="${REPO_ROOT}/target/.dev-up-provisioned"
RELAY_LOG="${REPO_ROOT}/target/dev-up-relay.log"

usage() {
  cat <<'EOF'
Usage: scripts/dev-up.sh [options]

Provisions the local Buzz dev environment on first run and starts it on every
run. Safe to re-run at any time: every step is idempotent.

Modes (at most one):
  (default)      Ensure the environment, then start relay + desktop (just dev)
  --relay        Ensure the environment, then start the relay only (just relay)
  --relay-only   Alias for --relay
  --web          Ensure the environment, start the relay in the background,
                 then run the web dev server (just web)
  --setup-only   Ensure the environment only: build nothing, start nothing

Options:
  --no-build     Skip the `just build` workspace build (fastest warm start)
  -y, --yes      Non-interactive: apply port remediations without prompting
  -h, --help     Show this help and exit

Ensuring the environment covers:
  * the Hermit-pinned toolchain on PATH, so bin/activate-hermit is not needed
  * Docker Desktop started and its daemon reachable
  * port conflicts cleared: a Homebrew Redis on 6379, a stale buzz-relay on
    3000 / 8080 / 9102
  * first run only: .env, Docker services, migrations, local community seed,
    JS dependencies and git hooks, all delegated to `just setup`
EOF
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --relay|--relay-only) MODE="relay" ;;
      --web) MODE="web" ;;
      --setup-only) MODE="setup-only" ;;
      --no-build) RUN_BUILD=false ;;
      -y|--yes) ASSUME_YES=true ;;
      -h|--help) usage; exit 0 ;;
      --) ;;
      *)
        error "Unknown option: $1"
        usage >&2
        exit 1
        ;;
    esac
    shift
  done
  if [[ "${MODE}" == "setup-only" ]]; then
    RUN_BUILD=false
  fi
}

on_error() {
  local code=$?
  local line="$1"
  error "Failed at line ${line}: ${BASH_COMMAND} (exit ${code})"
  if [[ -n "${RELAY_PGID}" ]]; then
    error "Relay output: ${RELAY_LOG}"
  fi
  error "Fix the problem above, then re-run: ${INVOCATION}"
}

confirm() {
  local prompt="$1"
  if [[ "${ASSUME_YES}" == true ]]; then
    return 0
  fi
  if [[ ! -t 0 ]]; then
    error "Not running on a terminal, so this cannot be confirmed interactively."
    error "Re-run with --yes to apply remediations automatically: ${INVOCATION} --yes"
    return 1
  fi
  local reply
  read -r -p "$(echo -e "${YELLOW}[dev-up]${NC} ${prompt} [y/N] ")" reply
  case "${reply}" in
    [yY]|[yY][eE][sS]) return 0 ;;
    *) return 1 ;;
  esac
}

# ---- Environment ------------------------------------------------------------

load_env() {
  if [[ ! -f "${REPO_ROOT}/.env" ]]; then
    return 0
  fi
  set -o allexport
  # shellcheck disable=SC1091
  source "${REPO_ROOT}/.env"
  set +o allexport
}

# Mirrors the port derivation in the Justfile's `dev` recipe.
resolve_ports() {
  local bind_addr="${BUZZ_BIND_ADDR:-0.0.0.0:3000}"
  RELAY_PORT="${bind_addr##*:}"
  if [[ -z "${RELAY_PORT}" ]]; then
    RELAY_PORT=3000
  fi
  HEALTH_PORT="${BUZZ_HEALTH_PORT:-8080}"
  METRICS_PORT="${BUZZ_METRICS_PORT:-9102}"
}

require_just() {
  if ! command -v just >/dev/null 2>&1; then
    error "just was not found even with ${REPO_ROOT}/bin on PATH."
    error "Check that the repo's Hermit bin/ directory is intact."
    exit 1
  fi
}

# ---- Docker -----------------------------------------------------------------

container_running() {
  local names
  names="$(docker ps --format '{{.Names}}' 2>/dev/null || true)"
  grep -qx "$1" <<<"${names}"
}

container_exists() {
  docker inspect --format '{{.Id}}' "$1" >/dev/null 2>&1
}

app_installed() {
  [[ -d "/Applications/$1.app" || -d "${HOME}/Applications/$1.app" ]]
}

use_runtime() {
  RUNTIME_LABEL="$1"
  RUNTIME_APP="$2"
}

# Docker Desktop is one of several runtimes behind the `docker` CLI, so the
# active context decides what to launch. `docker context show` and `... inspect`
# read local config only, so they still answer while the daemon is dead — which
# is the only time this matters.
resolve_container_runtime() {
  local context endpoint
  context="$(docker context show 2>/dev/null || true)"
  endpoint="$(docker context inspect --format '{{.Endpoints.docker.Host}}' 2>/dev/null || true)"

  case "${context}" in
    orbstack) use_runtime "OrbStack" "OrbStack"; return 0 ;;
    desktop-linux) use_runtime "Docker Desktop" "Docker"; return 0 ;;
    colima*) use_runtime "Colima" ""; return 0 ;;
  esac

  # A DOCKER_HOST override reports the context as `default`, which hides the
  # runtime's name; its socket path still identifies it.
  case "${endpoint}" in
    *orbstack*) use_runtime "OrbStack" "OrbStack"; return 0 ;;
    *colima*) use_runtime "Colima" ""; return 0 ;;
    */.docker/run/docker.sock) use_runtime "Docker Desktop" "Docker"; return 0 ;;
  esac

  if app_installed OrbStack; then
    use_runtime "OrbStack" "OrbStack"
  elif app_installed Docker; then
    use_runtime "Docker Desktop" "Docker"
  else
    use_runtime "" ""
  fi
  if [[ -n "${RUNTIME_LABEL}" ]]; then
    warn "Could not tell which runtime docker context '${context:-unknown}' points at; assuming ${RUNTIME_LABEL} because it is installed."
  fi
}

ensure_docker() {
  if ! command -v docker >/dev/null 2>&1; then
    error "The docker CLI was not found. Install a container runtime:"
    error "  Docker Desktop https://docs.docker.com/get-docker/ or OrbStack https://orbstack.dev"
    exit 1
  fi
  if docker info >/dev/null 2>&1; then
    return 0
  fi

  resolve_container_runtime
  if [[ -z "${RUNTIME_APP}" ]]; then
    error "The Docker daemon is not responding, and this script cannot start ${RUNTIME_LABEL:-the configured runtime} for you."
    error "Start it yourself, or point docker at a runtime that is already up:"
    error "  docker context ls    # then: docker context use <name>"
    exit 1
  fi
  if ! app_installed "${RUNTIME_APP}"; then
    error "The Docker daemon is not responding: docker is configured for ${RUNTIME_LABEL}, but ${RUNTIME_APP}.app is not installed."
    error "Install ${RUNTIME_LABEL}, or point docker at a runtime that is already up:"
    error "  docker context ls    # then: docker context use <name>"
    exit 1
  fi

  log "The ${RUNTIME_LABEL} daemon is not responding — starting ${RUNTIME_LABEL}..."
  if ! open -a "${RUNTIME_APP}"; then
    error "Could not launch ${RUNTIME_LABEL}. Start it manually, then re-run: ${INVOCATION}"
    exit 1
  fi

  local waited=0
  echo -e -n "${BLUE}[dev-up]${NC} Waiting for the ${RUNTIME_LABEL} daemon"
  while [[ ${waited} -lt ${DOCKER_WAIT_SECS} ]]; do
    if docker info >/dev/null 2>&1; then
      echo " ready"
      return 0
    fi
    echo -n "."
    sleep 3
    waited=$((waited + 3))
  done
  echo " timed out"
  # `open` exits 0 once launchd accepts the request, even when the app crashes
  # immediately after, so the poll above is the only evidence that counts.
  error "${RUNTIME_LABEL} was launched successfully but served no daemon within ${DOCKER_WAIT_SECS}s."
  error "A launch that reports success and leaves nothing running usually means a broken or"
  error "half-finished install — an interrupted update can leave the app bundle quarantined"
  error "and crashing on start. Options, cheapest first:"
  error "  1. start ${RUNTIME_LABEL} by hand and watch for a crash report"
  error "  2. switch to a runtime that works: docker context use <name>"
  error "  3. reinstall it: https://docs.docker.com/get-docker/ (Docker Desktop), https://orbstack.dev (OrbStack)"
  error "Configured contexts:"
  docker context ls >&2 || true
  exit 1
}

# ---- Port preflight ---------------------------------------------------------

port_listener_pids() {
  if ! command -v lsof >/dev/null 2>&1; then
    return 0
  fi
  lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null | sort -u || true
}

describe_listener() {
  lsof -nP +c 0 -iTCP:"$1" -sTCP:LISTEN 2>/dev/null || true
}

process_path() {
  ps -p "$1" -o comm= 2>/dev/null | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' || true
}

classify_listener() {
  local path base
  path="$(process_path "$1")"
  base="${path##*/}"
  # Whichever runtime publishes container ports owns them; never a kill target.
  # OrbStack publishes through "OrbStack Helper" inside its own app bundle.
  case "${path}" in
    */Docker.app/*|*com.docker*|*vpnkit*|*/OrbStack.app/*) echo "runtime"; return 0 ;;
  esac
  case "${base}" in
    buzz-relay) echo "buzz-relay"; return 0 ;;
    redis-server)
      case "${path}" in
        /opt/homebrew/*|/usr/local/*) echo "homebrew-redis"; return 0 ;;
      esac
      ;;
  esac
  echo "other"
}

wait_for_port_free() {
  local port="$1"
  local waited=0
  while [[ ${waited} -lt ${PORT_FREE_WAIT_SECS} ]]; do
    if [[ -z "$(port_listener_pids "${port}")" ]]; then
      return 0
    fi
    sleep 1
    waited=$((waited + 1))
  done
  [[ -z "$(port_listener_pids "${port}")" ]]
}

brew_redis_service() {
  local name
  name="$(brew services list 2>/dev/null | awk '$1 ~ /^redis(@[0-9.]+)?$/ && $2 != "none" { print $1; exit }' || true)"
  echo "${name:-redis}"
}

stop_homebrew_redis() {
  local pid="$1"
  local service
  warn "A Homebrew Redis (pid ${pid}) holds port 6379, so the compose buzz-redis service cannot bind it."
  if ! command -v brew >/dev/null 2>&1; then
    error "Homebrew is not on PATH, so it cannot be stopped automatically."
    error "Stop that Redis, then re-run: ${INVOCATION}"
    exit 1
  fi
  service="$(brew_redis_service)"
  if ! confirm "Stop it with 'brew services stop ${service}'?"; then
    error "Left running. Free port 6379 yourself, then re-run: ${INVOCATION}"
    exit 1
  fi
  log "Stopping Homebrew Redis: brew services stop ${service}"
  if ! brew services stop "${service}"; then
    error "'brew services stop ${service}' failed. Stop that Redis manually, then re-run: ${INVOCATION}"
    exit 1
  fi
  if ! wait_for_port_free 6379; then
    error "Port 6379 is still held after stopping ${service}:"
    describe_listener 6379 >&2
    exit 1
  fi
  success "Port 6379 is free"
}

stop_stale_relay() {
  local pid="$1"
  local port="$2"
  warn "A buzz-relay (pid ${pid}) already holds port ${port}:"
  describe_listener "${port}"
  if ! confirm "Stop that stale buzz-relay (pid ${pid})?"; then
    error "Left running. Stop pid ${pid} yourself, then re-run: ${INVOCATION}"
    exit 1
  fi
  log "Sending SIGTERM to pid ${pid}..."
  kill "${pid}" 2>/dev/null || true
  if wait_for_port_free "${port}"; then
    success "Port ${port} is free"
    return 0
  fi
  warn "pid ${pid} ignored SIGTERM — sending SIGKILL"
  kill -9 "${pid}" 2>/dev/null || true
  if ! wait_for_port_free "${port}"; then
    error "Port ${port} is still held:"
    describe_listener "${port}" >&2
    exit 1
  fi
  success "Port ${port} is free"
}

ensure_port_available() {
  local port="$1"
  local label="$2"
  local owner_container="${3:-}"
  if [[ -n "${owner_container}" ]] && container_running "${owner_container}"; then
    return 0
  fi
  local pids
  pids="$(port_listener_pids "${port}")"
  if [[ -z "${pids}" ]]; then
    return 0
  fi
  local pid
  for pid in ${pids}; do
    case "$(classify_listener "${pid}")" in
      runtime)
        # Reached only when the owning container is absent, so some other
        # container is publishing this port and compose will fail to bind it.
        warn "Port ${port} (${label}) is published by the container runtime for a different container."
        ;;
      homebrew-redis) stop_homebrew_redis "${pid}" ;;
      buzz-relay) stop_stale_relay "${pid}" "${port}" ;;
      *)
        error "Port ${port} (${label}) is held by a process this script will not touch:"
        describe_listener "${port}" >&2
        error "Free that port yourself, then re-run: ${INVOCATION}"
        exit 1
        ;;
    esac
  done
}

ensure_service_ports() {
  ensure_port_available 5432 "compose postgres" "buzz-postgres"
  ensure_port_available 6379 "compose redis" "buzz-redis"
}

ensure_relay_ports() {
  ensure_port_available "${RELAY_PORT}" "relay"
  ensure_port_available "${HEALTH_PORT}" "relay health"
  ensure_port_available "${METRICS_PORT}" "relay metrics"
}

# ---- Provisioning -----------------------------------------------------------

first_run_reason() {
  if [[ ! -f "${REPO_ROOT}/.env" ]]; then
    echo ".env is missing"
    return 0
  fi
  # Only desktop/ and web/ are checked, because those are the two installs
  # dev-setup.sh performs, each one behind the same guards: the directory has to
  # exist, and pnpm has to be on PATH or the install is skipped with a warning.
  # A root node_modules is not asserted: nothing in `just setup` creates it, so
  # requiring it would make every warm start look like a first run.
  if command -v pnpm >/dev/null 2>&1; then
    local project
    for project in desktop web; do
      if [[ -d "${REPO_ROOT}/${project}" && ! -d "${REPO_ROOT}/${project}/node_modules" ]]; then
        echo "${project} JS dependencies are missing (no ${project}/node_modules)"
        return 0
      fi
    done
  fi
  if ! container_exists buzz-postgres; then
    echo "the buzz-postgres container has never been created"
    return 0
  fi
  if ! container_exists buzz-redis; then
    echo "the buzz-redis container has never been created"
    return 0
  fi
}

# A populated `communities` table means both the migrations and the local
# community seed have run; the relay fails closed on unknown hosts without it.
db_provisioned() {
  if ! container_running buzz-postgres; then
    return 1
  fi
  local count
  count="$(docker exec -e PGPASSWORD="${PGPASSWORD:-buzz_dev}" buzz-postgres \
    psql -tAXq -U "${PGUSER:-buzz}" -d "${PGDATABASE:-buzz}" \
    -c 'SELECT count(*) FROM communities' 2>/dev/null | tr -d '[:space:]' || true)"
  case "${count}" in
    ''|*[!0-9]*|0) return 1 ;;
  esac
  return 0
}

mark_provisioned() {
  mkdir -p "$(dirname "${STAMP_FILE}")"
  date -u '+%Y-%m-%dT%H:%M:%SZ' > "${STAMP_FILE}"
}

ensure_provisioned() {
  local reason
  reason="$(first_run_reason)"
  if [[ -n "${reason}" ]]; then
    log "First run detected: ${reason}"
    log "Provisioning with 'just setup' (services, migrations, seed, JS deps, git hooks)..."
    just setup
    mark_provisioned
    success "Provisioning complete"
    return 0
  fi

  log "Checkout is already provisioned — ensuring Docker services are healthy..."
  just _ensure-services

  if [[ -f "${STAMP_FILE}" && "${MODE}" != "setup-only" ]]; then
    return 0
  fi
  if db_provisioned; then
    mark_provisioned
    return 0
  fi
  warn "Database migrations or the local community seed are missing."
  log "Applying them with 'just migrate'..."
  just migrate
  mark_provisioned
}

run_workspace_build() {
  if [[ "${RUN_BUILD}" != true ]]; then
    if [[ "${MODE}" != "setup-only" ]]; then
      log "Skipping the workspace build (--no-build)"
    fi
    return 0
  fi
  log "Building the Rust workspace (just build)..."
  just build
}

# ---- Start ------------------------------------------------------------------

print_verification() {
  echo ""
  log "Docker services:"
  docker compose ps || true
  echo ""
  log "Health endpoints (once the relay is running):"
  echo "    http://localhost:${RELAY_PORT}/health"
  echo "    http://localhost:${HEALTH_PORT}/_readiness"
  echo ""
}

stop_background_relay() {
  if [[ -z "${RELAY_PGID}" ]]; then
    return 0
  fi
  log "Stopping the background relay..."
  kill -TERM -- "-${RELAY_PGID}" 2>/dev/null || true
  RELAY_PGID=""
}

# `just relay` runs the relay under cargo, so terminating the recipe alone would
# orphan the relay itself. Job control gives the recipe its own process group,
# which lets the exit trap signal the whole tree.
start_background_relay() {
  mkdir -p "$(dirname "${RELAY_LOG}")"
  log "Starting the relay in the background (just relay), logging to ${RELAY_LOG}"
  set -m
  just relay >"${RELAY_LOG}" 2>&1 &
  RELAY_PGID=$!
  set +m
  trap stop_background_relay EXIT

  local waited=0
  echo -e -n "${BLUE}[dev-up]${NC} Waiting for the relay (a cold build takes a few minutes)"
  while [[ ${waited} -lt ${RELAY_READY_WAIT_SECS} ]]; do
    if ! kill -0 "${RELAY_PGID}" 2>/dev/null; then
      echo " exited"
      error "The relay exited during startup. Last lines of ${RELAY_LOG}:"
      tail -n 30 "${RELAY_LOG}" >&2 || true
      exit 1
    fi
    if curl --silent --fail --max-time 1 "http://127.0.0.1:${HEALTH_PORT}/_readiness" >/dev/null 2>&1; then
      echo " ready"
      return 0
    fi
    echo -n "."
    sleep 3
    waited=$((waited + 3))
  done
  echo " timed out"
  error "The relay was not ready within ${RELAY_READY_WAIT_SECS}s. See ${RELAY_LOG}"
  exit 1
}

start_selected_mode() {
  case "${MODE}" in
    setup-only)
      print_verification
      success "Environment is ready. Start it with: scripts/dev-up.sh"
      ;;
    relay)
      print_verification
      log "Starting the relay (just relay)..."
      exec just relay
      ;;
    web)
      start_background_relay
      log "Starting the web dev server (just web)..."
      just web
      ;;
    dev)
      log "Starting the relay and desktop app (just dev)..."
      exec just dev
      ;;
  esac
}

main() {
  if [[ $# -gt 0 ]]; then
    INVOCATION="scripts/dev-up.sh $*"
  fi
  parse_args "$@"

  cd "${REPO_ROOT}"
  # What the Justfile recipes do instead of sourcing bin/activate-hermit: the
  # Hermit shims in bin/ download the pinned toolchain on first use, and
  # activate-hermit targets interactive shells (it refuses to run unsourced).
  export PATH="${REPO_ROOT}/bin:$PATH"
  require_just
  load_env

  ensure_docker
  ensure_service_ports
  ensure_provisioned

  # `just setup` creates .env from .env.example on a first run, so the relay
  # ports are only knowable after provisioning.
  load_env
  resolve_ports
  if [[ "${MODE}" != "setup-only" ]]; then
    ensure_relay_ports
  fi

  run_workspace_build
  start_selected_mode
}

trap 'on_error "${LINENO}"' ERR

main "$@"
