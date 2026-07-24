#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="${repo_root}/scripts/check-local-services.sh"
compose_file="${repo_root}/docker-compose.yml"
test_tmp=$(mktemp -d)
trap 'rm -rf "${test_tmp}"' EXIT

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
      hang) sleep 3 ;;
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
      hang) sleep 3 ;;
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

promoted_output="${test_tmp}/promoted-output"
if PATH="${test_tmp}/bin:${PATH}" \
  BUZZ_REQUIRED_LOCAL_SERVICES=keycloak \
  MOCK_KEYCLOAK_STATE=unhealthy \
  "${checker}" --timeout 0 --interval 1 >"${promoted_output}" 2>&1; then
  echo "checker accepted an explicitly required unhealthy optional service" >&2
  exit 1
fi
grep -Fq "required keycloak: unhealthy" "${promoted_output}"

bounded_start_output=$(
  PATH="${test_tmp}/bin:${PATH}" \
    BUZZ_LOCAL_SERVICE_OPTIONAL_START_TIMEOUT_SECONDS=1 \
    MOCK_OPTIONAL_START_STATE=hang \
    "${checker}" --start --timeout 0 --interval 1 2>&1
)
grep -Fq "optional service startup timed out after 1s" <<<"${bounded_start_output}"

bounded_inspect_output=$(
  PATH="${test_tmp}/bin:${PATH}" \
    BUZZ_LOCAL_SERVICE_INSPECT_TIMEOUT_SECONDS=1 \
    MOCK_PROMETHEUS_STATE=hang \
    "${checker}" --timeout 0 --interval 1 2>&1
)
grep -Fq "optional prometheus: inspection timed out" <<<"${bounded_inspect_output}"

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
