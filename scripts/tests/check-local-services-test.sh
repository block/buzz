#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="${repo_root}/scripts/check-local-services.sh"
compose_file="${repo_root}/docker-compose.yml"
justfile="${repo_root}/Justfile"
dev_setup="${repo_root}/scripts/dev-setup.sh"
test_tmp=$(mktemp -d)

cleanup() {
  local pid_file pid
  for pid_file in "${test_tmp}"/*-process-pids; do
    [[ -f "${pid_file}" ]] || continue
    while IFS= read -r pid; do
      [[ "${pid}" =~ ^[0-9]+$ ]] || continue
      kill -KILL "${pid}" 2>/dev/null || true
    done <"${pid_file}"
  done
  rm -rf "${test_tmp}"
}
trap cleanup EXIT

mkdir -p "${test_tmp}/bin"
cat >"${test_tmp}/bin/docker" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
  compose)
    if [[ "$*" == *"postgres redis"* ]]; then
      start_state="${MOCK_REQUIRED_START_STATE:-success}"
    else
      start_state="${MOCK_OPTIONAL_START_STATE:-success}"
    fi
    case "${start_state}" in
      success) echo "mock compose startup complete" ;;
      failed)
        echo "mock compose startup failed" >&2
        exit 1
        ;;
      hang)
        trap '' TERM
        sleep 30 &
        child_pid=$!
        if [[ -n "${MOCK_PROCESS_PID_FILE:-}" ]]; then
          printf '%s\n%s\n' "$$" "${child_pid}" >"${MOCK_PROCESS_PID_FILE}"
        fi
        while true; do
          wait "${child_pid}" || true
          sleep 30 &
          child_pid=$!
        done
        ;;
      *)
        echo "unsupported mock start state: ${start_state}" >&2
        exit 64
        ;;
    esac
    ;;
  inspect)
    container="${*: -1}"
    service="${container#buzz-}"
    state_var="MOCK_$(printf '%s' "${service}" | tr '[:lower:]-' '[:upper:]_')_STATE"
    state="${!state_var:-}"

    if [[ -z "${state}" ]]; then
      case "${service}" in
        adminer | prometheus) state="running" ;;
        minio-init) state="completed" ;;
        *) state="healthy" ;;
      esac
    fi

    case "${state}" in
      healthy) printf 'running|healthy|0\n' ;;
      unhealthy) printf 'running|unhealthy|0\n' ;;
      starting) printf 'running|starting|0\n' ;;
      running) printf 'running|none|0\n' ;;
      completed) printf 'exited|none|0\n' ;;
      failed) printf 'exited|none|1\n' ;;
      missing) exit 1 ;;
      hang)
        trap '' TERM
        sleep 30 &
        child_pid=$!
        if [[ -n "${MOCK_PROCESS_PID_FILE:-}" ]]; then
          printf '%s\n%s\n' "$$" "${child_pid}" >"${MOCK_PROCESS_PID_FILE}"
        fi
        while true; do
          wait "${child_pid}" || true
          sleep 30 &
          child_pid=$!
        done
        ;;
      *)
        echo "unsupported mock state: ${state}" >&2
        exit 64
        ;;
    esac
    ;;
  *)
    echo "unexpected docker command: $*" >&2
    exit 64
    ;;
esac
MOCK
chmod +x "${test_tmp}/bin/docker"

if [[ ! -x "${checker}" ]]; then
  echo "local service checker is missing or not executable: ${checker}" >&2
  exit 1
fi

optional_output=$(
  PATH="${test_tmp}/bin:${PATH}" \
    MOCK_KEYCLOAK_STATE=unhealthy \
    "${checker}" --timeout 0 --interval 1 2>&1
)
grep -Fq "required postgres: healthy" <<<"${optional_output}"
grep -Fq "optional keycloak: unhealthy" <<<"${optional_output}"

required_output="${test_tmp}/required-output"
if PATH="${test_tmp}/bin:${PATH}" \
  MOCK_POSTGRES_STATE=unhealthy \
  "${checker}" --timeout 0 --interval 1 >"${required_output}" 2>&1; then
  echo "checker accepted an unhealthy required service" >&2
  exit 1
fi
grep -Fq "required postgres: unhealthy" "${required_output}"

completed_required_output="${test_tmp}/completed-required-output"
if PATH="${test_tmp}/bin:${PATH}" \
  MOCK_POSTGRES_STATE=completed \
  "${checker}" --timeout 0 --interval 1 >"${completed_required_output}" 2>&1; then
  echo "checker accepted a completed Postgres service" >&2
  exit 1
fi
grep -Fq "required postgres: completed" "${completed_required_output}"

running_required_output="${test_tmp}/running-required-output"
if PATH="${test_tmp}/bin:${PATH}" \
  MOCK_POSTGRES_STATE=running \
  "${checker}" --timeout 0 --interval 1 >"${running_required_output}" 2>&1; then
  echo "checker accepted Postgres without a healthy health check" >&2
  exit 1
fi
grep -Fq "required postgres: running" "${running_required_output}"

promoted_output="${test_tmp}/promoted-output"
if PATH="${test_tmp}/bin:${PATH}" \
  BUZZ_REQUIRED_LOCAL_SERVICES=keycloak \
  MOCK_KEYCLOAK_STATE=unhealthy \
  "${checker}" --timeout 0 --interval 1 >"${promoted_output}" 2>&1; then
  echo "checker accepted an explicitly required unhealthy optional service" >&2
  exit 1
fi
grep -Fq "required keycloak: unhealthy" "${promoted_output}"

promoted_completed_output="${test_tmp}/promoted-completed-output"
if PATH="${test_tmp}/bin:${PATH}" \
  BUZZ_REQUIRED_LOCAL_SERVICES=adminer \
  MOCK_ADMINER_STATE=completed \
  "${checker}" --timeout 0 --interval 1 >"${promoted_completed_output}" 2>&1; then
  echo "checker accepted a completed promoted long-running service" >&2
  exit 1
fi
grep -Fq "required adminer: completed" "${promoted_completed_output}"

PATH="${test_tmp}/bin:${PATH}" \
  BUZZ_REQUIRED_LOCAL_SERVICES=minio-init \
  "${checker}" --timeout 0 --interval 1 >/dev/null

bounded_start_output="${test_tmp}/bounded-start-output"
bounded_start_pids="${test_tmp}/bounded-start-process-pids"
PATH="${test_tmp}/bin:${PATH}" \
  BUZZ_LOCAL_SERVICE_OPTIONAL_START_TIMEOUT_SECONDS=1 \
  MOCK_OPTIONAL_START_STATE=hang \
  MOCK_PROCESS_PID_FILE="${bounded_start_pids}" \
  "${checker}" --start --timeout 0 --interval 1 >"${bounded_start_output}" 2>&1 &
bounded_checker_pid=$!
bounded_ticks=0
while kill -0 "${bounded_checker_pid}" 2>/dev/null && ((bounded_ticks < 30)); do
  sleep 0.1
  bounded_ticks=$((bounded_ticks + 1))
done
if kill -0 "${bounded_checker_pid}" 2>/dev/null; then
  kill -KILL "${bounded_checker_pid}" 2>/dev/null || true
  wait "${bounded_checker_pid}" 2>/dev/null || true
  echo "bounded checker exceeded the 3s hard test ceiling" >&2
  exit 1
fi
wait "${bounded_checker_pid}"
grep -Fq "optional service startup timed out after 1s" "${bounded_start_output}"
while IFS= read -r process_pid; do
  if kill -0 "${process_pid}" 2>/dev/null; then
    echo "bounded checker leaked descendant pid ${process_pid}" >&2
    exit 1
  fi
done <"${bounded_start_pids}"
rm -f "${bounded_start_pids}"

bounded_inspect_output=$(
  PATH="${test_tmp}/bin:${PATH}" \
    BUZZ_LOCAL_SERVICE_INSPECT_TIMEOUT_SECONDS=1 \
    MOCK_PROMETHEUS_STATE=hang \
    MOCK_PROCESS_PID_FILE="${test_tmp}/bounded-inspect-process-pids" \
    "${checker}" --timeout 0 --interval 1 2>&1
)
grep -Fq "optional prometheus: inspection timed out" <<<"${bounded_inspect_output}"
while IFS= read -r process_pid; do
  if kill -0 "${process_pid}" 2>/dev/null; then
    echo "bounded inspection leaked descendant pid ${process_pid}" >&2
    exit 1
  fi
done <"${test_tmp}/bounded-inspect-process-pids"
rm -f "${test_tmp}/bounded-inspect-process-pids"

ensure_services_block=$(
  sed -n '/^_ensure-services:/,/^# Apply database migrations/p' "${justfile}"
)
if grep -Eq 'docker (compose )?ps' <<<"${ensure_services_block}"; then
  echo "_ensure-services runs an unbounded Docker status fallback" >&2
  exit 1
fi

dev_setup_failure_block=$(
  sed -n '/if ! .*_ensure-services; then/,/^fi$/p' "${dev_setup}"
)
if grep -Eq 'docker (compose )?ps' <<<"${dev_setup_failure_block}"; then
  echo "dev-setup runs a redundant unbounded Docker status fallback" >&2
  exit 1
fi

keycloak_block=$(
  sed -n '/^  keycloak:/,/^  minio:/p' "${compose_file}"
)
grep -Eq 'KC_HEALTH_ENABLED:[[:space:]]*"?true"?' <<<"${keycloak_block}"
grep -Fq '"bash"' <<<"${keycloak_block}"
grep -Fq '"-c"' <<<"${keycloak_block}"
grep -Fq 'HEAD /health/ready HTTP/1.0' <<<"${keycloak_block}"
grep -Fq '/dev/tcp/localhost/9000' <<<"${keycloak_block}"
if grep -Fq '/dev/tcp/localhost/8080' <<<"${keycloak_block}"; then
  echo "Keycloak readiness still probes the application port" >&2
  exit 1
fi

echo "local service checker contract passed"
