#!/usr/bin/env bash
# =============================================================================
# triage-up.sh — Run the fibre engine that the desktop Inbox calls
# =============================================================================
# Usage: ./scripts/triage-up.sh [--heuristic] [--port N] [-d] [--stop] [--restart] [--yes]
#
# Puts the Hermit-pinned Node on PATH, clears a stale instance off the port, and
# starts triage-service. The desktop app reaches it at http://localhost:8787
# unless VITE_TRIAGE_API_URL says otherwise. Re-running is cheap and idempotent.
# =============================================================================
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SERVICE_DIR="${REPO_ROOT}/triage-service"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log()     { echo -e "${BLUE}[triage-up]${NC} $*"; }
success() { echo -e "${GREEN}[triage-up]${NC} $*"; }
warn()    { echo -e "${YELLOW}[triage-up]${NC} $*"; }
error()   { echo -e "${RED}[triage-up]${NC} $*" >&2; }

MODE="start"
BACKGROUND=false
ASSUME_YES=false
USE_LLM=true
PORT="${PORT:-8787}"
MODEL="${TRIAGE_MODEL:-}"
INVOCATION="scripts/triage-up.sh"

HEALTH_WAIT_SECS=15
PORT_FREE_WAIT_SECS=10

# /target is gitignored, so the log and pidfile stay out of `git status`.
LOG_FILE="${REPO_ROOT}/target/triage-service.log"
PID_FILE="${REPO_ROOT}/target/triage-service.pid"

usage() {
  cat <<'EOF'
Usage: scripts/triage-up.sh [options]

Starts the fibre engine for the desktop Inbox. Safe to re-run: a
stale instance of this service is replaced rather than duplicated.

Modes (at most one):
  (default)      Start in the foreground; Ctrl-C stops it
  -d, --background   Start detached, logging to target/triage-service.log
  --restart      Stop a running instance, then start
  --stop         Stop a running instance and exit
  --status       Report whether the service is up, then exit

Options:
  --heuristic    Classify with the built-in heuristic instead of an LLM. The
                 heuristic cannot judge importance in context, so triage is
                 much blunter — this is the offline fallback, not parity.
  --llm          Classify with an LLM (the default; needs OPENAI_API_KEY)
  --model NAME   Model to classify with (default gpt-4o-mini)
  --port N       Listen on N instead of 8787. The desktop app defaults to
                 8787, so set VITE_TRIAGE_API_URL to match if you change it.
  -y, --yes      Non-interactive: replace a stale instance without prompting
  -h, --help     Show this help and exit
EOF
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      -d|--background) BACKGROUND=true ;;
      --restart) MODE="restart" ;;
      --stop) MODE="stop" ;;
      --status) MODE="status" ;;
      --llm) USE_LLM=true ;;
      --heuristic) USE_LLM=false ;;
      --model)
        shift || { error "--model needs a value"; exit 1; }
        MODEL="$1"
        ;;
      --port)
        shift || { error "--port needs a value"; exit 1; }
        PORT="$1"
        ;;
      -y|--yes) ASSUME_YES=true ;;
      -h|--help) usage; exit 0 ;;
      *)
        error "Unknown option: $1"
        usage >&2
        exit 1
        ;;
    esac
    shift
  done
}

on_error() {
  local code=$?
  error "Failed at line $1: ${BASH_COMMAND} (exit ${code})"
  error "Fix the problem above, then re-run: ${INVOCATION}"
}

confirm() {
  local prompt="$1"
  if [[ "${ASSUME_YES}" == true ]]; then
    return 0
  fi
  if [[ ! -t 0 ]]; then
    error "Not running on a terminal, so this cannot be confirmed interactively."
    error "Re-run with --yes to replace it automatically: ${INVOCATION} --yes"
    return 1
  fi
  local reply
  read -r -p "$(echo -e "${YELLOW}[triage-up]${NC} ${prompt} [y/N] ")" reply
  case "${reply}" in
    [yY]|[yY][eE][sS]) return 0 ;;
    *) return 1 ;;
  esac
}

# ---- Preconditions ----------------------------------------------------------

require_service_dir() {
  if [[ ! -f "${SERVICE_DIR}/server.mjs" ]]; then
    error "No triage service at ${SERVICE_DIR}/server.mjs"
    error "This script expects it at the repo root: triage-service/"
    exit 1
  fi
}

require_node() {
  if ! command -v node >/dev/null 2>&1; then
    error "node was not found even with ${REPO_ROOT}/bin on PATH."
    error "Check that the repo's Hermit bin/ directory is intact."
    exit 1
  fi
}

require_llm_key() {
  if [[ "${USE_LLM}" != true ]]; then
    return 0
  fi
  if [[ -z "${OPENAI_API_KEY:-}" ]]; then
    error "LLM classification is the default and needs OPENAI_API_KEY."
    error "Export it, or run with --heuristic for the blunter offline mode."
    exit 1
  fi
}

# ---- Process control --------------------------------------------------------

# These helpers report through stdout and always succeed. A bare `return 1` as
# control flow would fire the ERR trap, because `set -E` inherits it into every
# function even when the caller guards the call with `if`.

# Echoes "up" when the service answers its health probe.
service_state() {
  if curl --silent --fail --max-time 2 "http://127.0.0.1:${PORT}/health" \
    >/dev/null 2>&1; then
    echo "up"
  fi
}

port_listener_pids() {
  if command -v lsof >/dev/null 2>&1; then
    lsof -nP -iTCP:"${PORT}" -sTCP:LISTEN -t 2>/dev/null | sort -u || true
  fi
}

# The service is started with triage-service/ as its working directory, so its
# command line is a bare `node server.mjs` — matching the full path would never
# hit. Combined with "is listening on our port", `server.mjs` is specific enough.
pid_is_our_service() {
  if ps -p "$1" -o command= 2>/dev/null | grep -q "server\.mjs"; then
    return 0
  fi
  return 1
}

# Echoes the pid of a running instance, or nothing when none can be identified.
find_service_pid() {
  local pid
  if [[ -f "${PID_FILE}" ]]; then
    pid="$(cat "${PID_FILE}" 2>/dev/null || true)"
    if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
      echo "${pid}"
      return 0
    fi
  fi
  for pid in $(port_listener_pids); do
    if pid_is_our_service "${pid}"; then
      echo "${pid}"
      return 0
    fi
  done
}

# Echoes "free" once nothing listens on the port.
wait_for_port_free() {
  local waited=0
  while [[ ${waited} -lt ${PORT_FREE_WAIT_SECS} ]]; do
    if [[ -z "$(port_listener_pids)" && -z "$(service_state)" ]]; then
      echo "free"
      return 0
    fi
    sleep 1
    waited=$((waited + 1))
  done
}

stop_service() {
  local pid
  pid="$(find_service_pid)"

  if [[ -z "${pid}" ]]; then
    if [[ -n "$(service_state)" ]]; then
      error "Something is serving ${PORT} but its process could not be identified."
      error "Stop it yourself, then re-run: ${INVOCATION}"
      exit 1
    fi
    log "No triage service is running on port ${PORT}"
    rm -f "${PID_FILE}"
    return 0
  fi

  log "Stopping the triage service (pid ${pid})..."
  kill "${pid}" 2>/dev/null || true
  if [[ -z "$(wait_for_port_free)" ]]; then
    warn "pid ${pid} ignored SIGTERM — sending SIGKILL"
    kill -9 "${pid}" 2>/dev/null || true
    if [[ -z "$(wait_for_port_free)" ]]; then
      error "Port ${PORT} is still held:"
      lsof -nP +c 0 -iTCP:"${PORT}" -sTCP:LISTEN >&2 2>/dev/null || true
      exit 1
    fi
  fi
  rm -f "${PID_FILE}"
  success "Stopped"
}

# A foreign listener is a hard stop: replacing it could kill something that
# matters, and binding alongside it is impossible.
ensure_port_available() {
  local pids
  pids="$(port_listener_pids)"
  if [[ -z "${pids}" ]]; then
    return 0
  fi

  local pid
  for pid in ${pids}; do
    if ! pid_is_our_service "${pid}"; then
      error "Port ${PORT} is held by a process this script will not touch:"
      lsof -nP +c 0 -iTCP:"${PORT}" -sTCP:LISTEN >&2 2>/dev/null || true
      error "Free that port yourself or pass --port N, then re-run: ${INVOCATION}"
      exit 1
    fi
  done

  if [[ "${MODE}" != "restart" ]]; then
    warn "A triage service already holds port ${PORT} (pid ${pids//$'\n'/, })."
    if ! confirm "Replace it?"; then
      error "Left running. Use --status to inspect it, or --restart to replace it."
      exit 1
    fi
  fi
  stop_service
}

# ---- Start ------------------------------------------------------------------

describe_mode() {
  if [[ "${USE_LLM}" == true ]]; then
    echo "LLM (${MODEL:-gpt-4o-mini})"
  else
    echo "heuristic"
  fi
}

export_service_env() {
  export PORT="${PORT}"
  if [[ "${USE_LLM}" == true ]]; then
    export TRIAGE_LLM=1
    if [[ -n "${MODEL}" ]]; then
      export TRIAGE_MODEL="${MODEL}"
    fi
  else
    # The service treats "0" as an explicit opt-out; anything else with a key
    # present means LLM, so the flag has to be set rather than unset.
    export TRIAGE_LLM=0
  fi
}

# Echoes "ready" once the service answers, or nothing if it never does.
wait_for_health() {
  local waited=0
  echo -e -n "${BLUE}[triage-up]${NC} Waiting for the service" >&2
  while [[ ${waited} -lt ${HEALTH_WAIT_SECS} ]]; do
    if [[ -n "$(service_state)" ]]; then
      echo " ready" >&2
      echo "ready"
      return 0
    fi
    echo -n "." >&2
    sleep 1
    waited=$((waited + 1))
  done
  echo " timed out" >&2
}

print_ready() {
  echo ""
  success "Triage backend is up: http://localhost:${PORT} ($(describe_mode) mode)"
  if [[ "${PORT}" != "8787" ]]; then
    warn "The desktop app defaults to port 8787. Start it with"
    warn "  VITE_TRIAGE_API_URL=http://localhost:${PORT} just dev"
  fi
  echo ""
}

start_background() {
  mkdir -p "$(dirname "${LOG_FILE}")"
  log "Starting the triage service in the background ($(describe_mode) mode)"
  log "Logging to ${LOG_FILE}"

  # nohup keeps it alive when the launching terminal closes; disown drops it
  # from this shell's job table so exiting does not signal it.
  local pid
  cd "${SERVICE_DIR}"
  nohup node server.mjs >"${LOG_FILE}" 2>&1 &
  pid=$!
  disown "${pid}" 2>/dev/null || true
  echo "${pid}" >"${PID_FILE}"
  cd "${REPO_ROOT}"

  if [[ -z "$(wait_for_health)" ]]; then
    error "The service did not become healthy. Last lines of ${LOG_FILE}:"
    tail -n 20 "${LOG_FILE}" >&2 || true
    exit 1
  fi
  print_ready
  log "Stop it with: scripts/triage-up.sh --stop"
}

start_foreground() {
  log "Starting the triage service ($(describe_mode) mode) — Ctrl-C to stop"
  print_ready
  cd "${SERVICE_DIR}"
  exec node server.mjs
}

# ---- Main -------------------------------------------------------------------

main() {
  if [[ $# -gt 0 ]]; then
    INVOCATION="scripts/triage-up.sh $*"
  fi
  parse_args "$@"

  cd "${REPO_ROOT}"
  # Same approach as the Justfile recipes: the Hermit shims in bin/ download the
  # pinned toolchain on first use, and activate-hermit refuses to run unsourced.
  export PATH="${REPO_ROOT}/bin:$PATH"
  require_service_dir
  require_node

  case "${MODE}" in
    status)
      if [[ -z "$(service_state)" ]]; then
        log "No triage service is running on port ${PORT}"
        exit 1
      fi
      local pid
      pid="$(find_service_pid)"
      success "Triage backend is up on port ${PORT}${pid:+ (pid ${pid})}"
      return 0
      ;;
    stop)
      stop_service
      return 0
      ;;
  esac

  require_llm_key
  ensure_port_available
  export_service_env

  if [[ "${BACKGROUND}" == true ]]; then
    start_background
  else
    start_foreground
  fi
}

trap 'on_error "${LINENO}"' ERR

main "$@"
