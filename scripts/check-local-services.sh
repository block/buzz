#!/usr/bin/env bash
set -euo pipefail

default_required_services=(postgres redis)
default_optional_services=(adminer keycloak minio minio-init prometheus memory)
# The Memory image and protected token secret are provisioned by the separate
# AgentMemory phase. Keep it observable/promotable here without making normal
# Buzz `--start` claim or require that deployment.
default_start_optional_services=(adminer keycloak minio minio-init prometheus)
required_services=("${default_required_services[@]}")
timeout_seconds="${BUZZ_LOCAL_SERVICES_TIMEOUT_SECONDS:-120}"
interval_seconds="${BUZZ_LOCAL_SERVICES_POLL_SECONDS:-3}"
required_start_timeout_seconds="${BUZZ_LOCAL_SERVICE_REQUIRED_START_TIMEOUT_SECONDS:-120}"
optional_start_timeout_seconds="${BUZZ_LOCAL_SERVICE_OPTIONAL_START_TIMEOUT_SECONDS:-30}"
inspect_timeout_seconds="${BUZZ_LOCAL_SERVICE_INSPECT_TIMEOUT_SECONDS:-5}"
start_services=false

usage() {
  cat <<'EOF'
Usage: scripts/check-local-services.sh [options]

Wait for required local services and report optional service states.

Options:
  --start            Start required and optional Compose services before checks
  --require SERVICE  Promote an optional service to required (repeatable)
  --timeout SECONDS  Maximum wait for required services (default: 120)
  --interval SECONDS Poll interval while waiting (default: 3)
  -h, --help          Show this help

BUZZ_REQUIRED_LOCAL_SERVICES may contain comma- or space-separated optional
service names to promote to required, for example:
  BUZZ_REQUIRED_LOCAL_SERVICES=keycloak scripts/check-local-services.sh
EOF
}

contains_service() {
  local needle="$1"
  shift
  local service
  for service in "$@"; do
    if [[ "${service}" == "${needle}" ]]; then
      return 0
    fi
  done
  return 1
}

is_known_service() {
  contains_service "$1" \
    "${default_required_services[@]}" \
    "${default_optional_services[@]}"
}

require_service() {
  local service="$1"
  if ! is_known_service "${service}"; then
    echo "[local-services] unknown service cannot be required: ${service}" >&2
    exit 2
  fi
  if ! contains_service "${service}" "${required_services[@]}"; then
    required_services+=("${service}")
  fi
}

extra_required="${BUZZ_REQUIRED_LOCAL_SERVICES:-}"
extra_required="${extra_required//,/ }"
if [[ -n "${extra_required//[[:space:]]/}" ]]; then
  read -r -a extra_required_services <<<"${extra_required}"
  for service in "${extra_required_services[@]}"; do
    require_service "${service}"
  done
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --start)
      start_services=true
      shift
      ;;
    --require)
      if [[ $# -lt 2 ]]; then
        echo "[local-services] --require needs a service name" >&2
        exit 2
      fi
      require_service "$2"
      shift 2
      ;;
    --timeout)
      if [[ $# -lt 2 ]]; then
        echo "[local-services] --timeout needs a value" >&2
        exit 2
      fi
      timeout_seconds="$2"
      shift 2
      ;;
    --interval)
      if [[ $# -lt 2 ]]; then
        echo "[local-services] --interval needs a value" >&2
        exit 2
      fi
      interval_seconds="$2"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "[local-services] unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ ! "${timeout_seconds}" =~ ^[0-9]+$ ]]; then
  echo "[local-services] timeout must be a non-negative integer" >&2
  exit 2
fi
if [[ ! "${interval_seconds}" =~ ^[1-9][0-9]*$ ]]; then
  echo "[local-services] interval must be a positive integer" >&2
  exit 2
fi
for timeout_value in \
  "${required_start_timeout_seconds}" \
  "${optional_start_timeout_seconds}" \
  "${inspect_timeout_seconds}"; do
  if [[ ! "${timeout_value}" =~ ^[1-9][0-9]*$ ]]; then
    echo "[local-services] command timeouts must be positive integers" >&2
    exit 2
  fi
done

runtime_tmp=$(mktemp -d)
trap 'rm -rf "${runtime_tmp}"' EXIT

run_bounded() (
  set -m
  local output_file="$1"
  local limit_seconds="$2"
  shift 2

  "$@" >"${output_file}" 2>&1 &
  local command_pid=$!
  set +m
  local elapsed_ticks=0
  local max_ticks=$((limit_seconds * 10))

  while kill -0 "${command_pid}" 2>/dev/null; do
    if ((elapsed_ticks >= max_ticks)); then
      kill -TERM -- "-${command_pid}" 2>/dev/null || true

      local grace_ticks=0
      while kill -0 -- "-${command_pid}" 2>/dev/null && ((grace_ticks < 5)); do
        sleep 0.1
        grace_ticks=$((grace_ticks + 1))
      done

      if kill -0 -- "-${command_pid}" 2>/dev/null; then
        kill -KILL -- "-${command_pid}" 2>/dev/null || true
      fi

      local reap_ticks=0
      local process_state
      while ((reap_ticks < 5)); do
        process_state=$(ps -o stat= -p "${command_pid}" 2>/dev/null | tr -d '[:space:]')
        if [[ -z "${process_state}" || "${process_state}" == Z* ]]; then
          wait "${command_pid}" 2>/dev/null || true
          break
        fi
        sleep 0.1
        reap_ticks=$((reap_ticks + 1))
      done
      if ((reap_ticks >= 5)); then
        disown -a 2>/dev/null || true
      fi
      return 124
    fi
    sleep 0.1
    elapsed_ticks=$((elapsed_ticks + 1))
  done

  wait "${command_pid}"
)

start_local_services() {
  local required_output="${runtime_tmp}/required-start"
  local optional_output="${runtime_tmp}/optional-start"
  local start_status

  echo "[local-services] starting required Compose services"
  if run_bounded \
    "${required_output}" \
    "${required_start_timeout_seconds}" \
    docker compose up -d "${default_required_services[@]}"; then
    cat "${required_output}"
  else
    start_status=$?
    if [[ ${start_status} -eq 124 ]]; then
      echo "[local-services] required service startup timed out after ${required_start_timeout_seconds}s" >&2
    else
      echo "[local-services] required service startup failed (exit ${start_status})" >&2
    fi
    cat "${required_output}" >&2
    return 1
  fi

  echo "[local-services] starting optional Compose services"
  if run_bounded \
    "${optional_output}" \
    "${optional_start_timeout_seconds}" \
    docker compose up -d "${default_start_optional_services[@]}"; then
    cat "${optional_output}"
  else
    start_status=$?
    if [[ ${start_status} -eq 124 ]]; then
      echo "[local-services] optional service startup timed out after ${optional_start_timeout_seconds}s; continuing with required services" >&2
    else
      echo "[local-services] optional service startup failed (exit ${start_status}); continuing with required services" >&2
    fi
    cat "${optional_output}" >&2
  fi
}

if [[ "${start_services}" == "true" ]]; then
  start_local_services
fi

service_state() {
  local service="$1"
  local inspection
  local inspect_output="${runtime_tmp}/inspect-${service}"
  local inspect_status
  if run_bounded \
    "${inspect_output}" \
    "${inspect_timeout_seconds}" \
    docker inspect \
      --format '{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}|{{.State.ExitCode}}' \
      "buzz-${service}"; then
    inspection=$(<"${inspect_output}")
  else
    inspect_status=$?
    if [[ ${inspect_status} -eq 124 ]]; then
      printf 'inspection timed out\n'
    else
      printf 'missing\n'
    fi
    return 0
  fi

  local container_state health_state exit_code
  IFS='|' read -r container_state health_state exit_code <<<"${inspection}"
  case "${container_state}:${health_state}:${exit_code}" in
    running:healthy:*) printf 'healthy\n' ;;
    running:starting:*) printf 'starting\n' ;;
    running:unhealthy:*) printf 'unhealthy\n' ;;
    running:none:*) printf 'running\n' ;;
    exited:*:0) printf 'completed\n' ;;
    exited:*:*) printf 'failed (exit ${exit_code:-unknown})\n' ;;
    *) printf '%s\n' "${container_state:-unknown}" ;;
  esac
}

state_is_ready() {
  local service="$1"
  local state="$2"
  case "${service}:${state}" in
    postgres:healthy | redis:healthy | keycloak:healthy | minio:healthy | memory:healthy)
      return 0
      ;;
    adminer:running | prometheus:running | minio-init:completed)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

all_services=(
  "${default_required_services[@]}"
  "${default_optional_services[@]}"
)
deadline=$((SECONDS + timeout_seconds))

while true; do
  required_not_ready=()

  for service in "${all_services[@]}"; do
    state=$(service_state "${service}")
    if contains_service "${service}" "${required_services[@]}"; then
      role="required"
      if ! state_is_ready "${service}" "${state}"; then
        required_not_ready+=("${service}=${state}")
      fi
    else
      role="optional"
    fi
    printf '[local-services] %s %s: %s\n' "${role}" "${service}" "${state}"
  done

  if [[ ${#required_not_ready[@]} -eq 0 ]]; then
    echo "[local-services] all required services are ready"
    exit 0
  fi

  if ((SECONDS >= deadline)); then
    printf '[local-services] required services not ready after %ss: %s\n' \
      "${timeout_seconds}" "${required_not_ready[*]}" >&2
    exit 1
  fi

  remaining=$((deadline - SECONDS))
  sleep_for="${interval_seconds}"
  if ((sleep_for > remaining)); then
    sleep_for="${remaining}"
  fi
  echo "[local-services] waiting ${sleep_for}s for required services..."
  sleep "${sleep_for}"
done
