#!/usr/bin/env bash
# Regression checks for configurable host ports in the root dev Compose stack.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib/dev-service-env.sh"

assert_eq() {
  local actual="$1"
  local expected="$2"
  local description="$3"
  if [[ "${actual}" != "${expected}" ]]; then
    echo "${description}: expected ${expected}, got ${actual}" >&2
    exit 1
  fi
}

validate_local_service_port "Postgres" "postgres://buzz:secret@localhost:15432/buzz" "15432" "5432"
validate_local_service_port "Redis" "redis://127.0.0.1:16379" "16379" "6379"
validate_local_service_port "MinIO" "http://[::1]:19000" "19000" "80"
validate_local_service_port "External Postgres" "postgres://db.example.com:5432/buzz" "15432" "5432"

local_aliases=(
  "localhost"
  "localhost."
  "agent.localhost"
  "agent.localhost."
  "127.0.0.1"
  "127.1"
  "127.255.255.254"
  "2130706433"
  "0177.0.0.1"
  "0x7f000001"
  "[::1]"
  "[::ffff:127.0.0.1]"
  "[::ffff:7f00:1]"
)
for host in "${local_aliases[@]}"; do
  validate_local_service_port \
    "Postgres" \
    "postgres://buzz:secret@${host}:15432/buzz" \
    "15432" \
    "5432"
done

assert_validation_rejected() {
  local description="$1"
  local url="$2"
  local expected_port="$3"
  local default_url_port="$4"
  local validation_error="${TMP_DIR}/validation-error"

  if validate_local_service_port \
    "${description}" \
    "${url}" \
    "${expected_port}" \
    "${default_url_port}" 2> "${validation_error}"; then
    echo "${description} unexpectedly passed validation" >&2
    exit 1
  fi
  if grep -q 'do-not-print' "${validation_error}"; then
    echo "${description} leaked URL credentials" >&2
    exit 1
  fi
}

for host in "${local_aliases[@]}"; do
  assert_validation_rejected \
    "mismatched local alias ${host}" \
    "postgres://buzz:do-not-print@${host}:5432/buzz" \
    "15432" \
    "5432"
done

assert_validation_rejected \
  "unspecified IPv4 address" \
  "postgres://buzz:do-not-print@0.0.0.0:15432/buzz" \
  "15432" \
  "5432"
assert_validation_rejected \
  "unspecified IPv6 address" \
  "postgres://buzz:do-not-print@[::]:15432/buzz" \
  "15432" \
  "5432"
assert_validation_rejected \
  "IPv4-mapped unspecified address" \
  "postgres://buzz:do-not-print@[::ffff:0.0.0.0]:15432/buzz" \
  "15432" \
  "5432"

# Special-scheme URLs use their protocol default when the port is omitted.
validate_local_service_port "HTTPS MinIO" "https://localhost" "443" "80"

# The shared relay launcher may honor Compose host-port overrides, but it must
# never apply schemas to or start against service URLs and credentials inherited
# from a developer's environment.
(
  # shellcheck disable=SC2329
  docker() {
    case "$*" in
      "compose port postgres 5432") echo "127.0.0.1:15432" ;;
      "compose port redis 6379") echo "127.0.0.1:16379" ;;
      "compose port minio 9000") echo "127.0.0.1:19000" ;;
      *) echo "unexpected docker invocation: $*" >&2; return 1 ;;
    esac
  }

  export DATABASE_URL="postgres://remote:secret@db.example.com:5432/production"
  export PGHOST="db.example.com"
  export PGPORT="5432"
  export PGUSER="remote"
  export PGPASSWORD="secret"
  export PGDATABASE="production"
  export PGSCHEMA_PLAN_HOST="db.example.com"
  export PGSCHEMA_PLAN_PORT="5432"
  export PGSCHEMA_PLAN_DB="production"
  export PGSCHEMA_PLAN_USER="remote"
  export PGSCHEMA_PLAN_PASSWORD="secret"
  export REDIS_URL="redis://cache.example.com:6379"
  export BUZZ_S3_ENDPOINT="https://storage.example.com"
  export BUZZ_S3_ACCESS_KEY="remote-key"
  export BUZZ_S3_SECRET_KEY="remote-secret"
  export BUZZ_S3_BUCKET="production"

  configure_local_compose_service_env

  assert_eq "${DATABASE_URL}" "postgres://buzz:buzz_dev@localhost:15432/buzz" "test relay database URL"
  assert_eq "${PGSCHEMA_PLAN_HOST}" "localhost" "schema plan host"
  assert_eq "${PGSCHEMA_PLAN_PORT}" "15432" "schema plan port"
  assert_eq "${PGSCHEMA_PLAN_DB}" "buzz" "schema plan database"
  assert_eq "${PGSCHEMA_PLAN_USER}" "buzz" "schema plan user"
  assert_eq "${PGSCHEMA_PLAN_PASSWORD}" "buzz_dev" "schema plan password"
  assert_eq "${REDIS_URL}" "redis://localhost:16379" "test relay Redis URL"
  assert_eq "${BUZZ_S3_ENDPOINT}" "http://localhost:19000" "test relay MinIO endpoint"
  assert_eq "${BUZZ_S3_ACCESS_KEY}" "buzz_dev" "test relay MinIO access key"
  assert_eq "${BUZZ_S3_SECRET_KEY}" "buzz_dev_secret" "test relay MinIO secret key"
  assert_eq "${BUZZ_S3_BUCKET}" "buzz-media" "test relay MinIO bucket"
)

render_config() {
  local output="$1"
  shift
  env \
    -u PGPORT \
    -u REDIS_PORT \
    -u ADMINER_PORT \
    -u KEYCLOAK_PORT \
    -u MINIO_API_PORT \
    -u MINIO_CONSOLE_PORT \
    -u PROMETHEUS_PORT \
    -u PGUSER \
    -u PGPASSWORD \
    -u PGDATABASE \
    -u BUZZ_S3_ACCESS_KEY \
    -u BUZZ_S3_SECRET_KEY \
    -u BUZZ_S3_BUCKET \
    "$@" docker compose \
      --project-directory "${REPO_ROOT}" \
      --env-file /dev/null \
      config --format json > "${output}"
}

assert_config() {
  local config="$1"
  shift
  # shellcheck disable=SC2016
  node -e '
    const fs = require("node:fs");
    const config = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    const expected = JSON.parse(process.argv[2]);

    for (const [service, ports] of Object.entries(expected)) {
      const actual = (config.services[service].ports ?? []).map((port) => ({
        host_ip: port.host_ip,
        published: String(port.published),
        target: Number(port.target),
      }));
      if (JSON.stringify(actual) !== JSON.stringify(ports)) {
        throw new Error(`${service} ports: expected ${JSON.stringify(ports)}, got ${JSON.stringify(actual)}`);
      }
    }

    const postgres = config.services.postgres.environment;
    if (postgres.POSTGRES_USER !== "buzz" || postgres.POSTGRES_PASSWORD !== "buzz_dev" || postgres.POSTGRES_DB !== "buzz") {
      throw new Error("host-port overrides must not change Postgres initialization credentials");
    }

    const minio = config.services.minio.environment;
    if (minio.MINIO_ROOT_USER !== "buzz_dev" || minio.MINIO_ROOT_PASSWORD !== "buzz_dev_secret") {
      throw new Error("host-port overrides must not change MinIO initialization credentials");
    }
  ' "$config" "$1"
}

default_config="${TMP_DIR}/default.json"
render_config "${default_config}"
assert_config "${default_config}" '{
  "postgres":[{"host_ip":"127.0.0.1","published":"5432","target":5432}],
  "redis":[{"host_ip":"127.0.0.1","published":"6379","target":6379}],
  "adminer":[{"host_ip":"127.0.0.1","published":"8082","target":8080}],
  "keycloak":[{"host_ip":"127.0.0.1","published":"8180","target":8080}],
  "minio":[
    {"host_ip":"127.0.0.1","published":"9000","target":9000},
    {"host_ip":"127.0.0.1","published":"9001","target":9001}
  ],
  "prometheus":[{"host_ip":"127.0.0.1","published":"9090","target":9090}]
}'

custom_config="${TMP_DIR}/custom.json"
render_config "${custom_config}" \
  PGPORT=15432 \
  REDIS_PORT=16379 \
  ADMINER_PORT=18082 \
  KEYCLOAK_PORT=18180 \
  MINIO_API_PORT=19000 \
  MINIO_CONSOLE_PORT=19001 \
  PROMETHEUS_PORT=19090 \
  PGUSER=must-not-propagate \
  PGPASSWORD=must-not-propagate \
  PGDATABASE=must-not-propagate \
  BUZZ_S3_ACCESS_KEY=must-not-propagate \
  BUZZ_S3_SECRET_KEY=must-not-propagate \
  BUZZ_S3_BUCKET=must-not-propagate
assert_config "${custom_config}" '{
  "postgres":[{"host_ip":"127.0.0.1","published":"15432","target":5432}],
  "redis":[{"host_ip":"127.0.0.1","published":"16379","target":6379}],
  "adminer":[{"host_ip":"127.0.0.1","published":"18082","target":8080}],
  "keycloak":[{"host_ip":"127.0.0.1","published":"18180","target":8080}],
  "minio":[
    {"host_ip":"127.0.0.1","published":"19000","target":9000},
    {"host_ip":"127.0.0.1","published":"19001","target":9001}
  ],
  "prometheus":[{"host_ip":"127.0.0.1","published":"19090","target":9090}]
}'

docker_calls="${TMP_DIR}/docker-calls"
mock_bin="${TMP_DIR}/bin"
mkdir -p "${mock_bin}"
# shellcheck disable=SC2016
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'if [[ "$1" == "inspect" ]]; then' \
  '  echo healthy' \
  'elif [[ "$1" == "compose" ]]; then' \
  '  printf "%s\n" "$*" >> "${DOCKER_CALLS}"' \
  'else' \
  '  echo "unexpected docker invocation: $*" >&2' \
  '  exit 1' \
  'fi' > "${mock_bin}/docker"
chmod +x "${mock_bin}/docker"

PATH="${mock_bin}:${PATH}" DOCKER_CALLS="${docker_calls}" \
  "${REPO_ROOT}/bin/just" --justfile "${REPO_ROOT}/Justfile" _ensure-services

if ! grep -qx 'compose up -d' "${docker_calls}"; then
  echo "_ensure-services did not reconcile Compose when existing services were healthy" >&2
  exit 1
fi

# `just setup` runs bootstrap in a separate recipe, so dev-setup itself must
# add the repository's Hermit shims before validating URLs with node. Exercise
# that boundary without a system node binary or a real Docker daemon.
setup_probe_root="${TMP_DIR}/setup-probe"
mkdir -p "${setup_probe_root}/scripts/lib" "${setup_probe_root}/bin" "${setup_probe_root}/mock-bin"
cp "${REPO_ROOT}/scripts/dev-setup.sh" "${setup_probe_root}/scripts/dev-setup.sh"
cp "${REPO_ROOT}/scripts/lib/dev-service-env.sh" "${setup_probe_root}/scripts/lib/dev-service-env.sh"
cp "${REPO_ROOT}/bin/node" "${setup_probe_root}/bin/node"

cat > "${setup_probe_root}/mock-hermit" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

# Mimic the generated Hermit node shim closely enough for the URL parser used
# by dev-service-env.sh. The direct helper matrix above exercises real Node.
url="${7}"
default_port="${8}"
case "${url}" in
  postgres://*@localhost:5432/*) printf 'local\t5432' ;;
  redis://localhost:6379) printf 'local\t6379' ;;
  http://localhost:9000) printf 'local\t9000' ;;
  *) printf 'local\t%s' "${default_port}" ;;
esac
EOF
chmod +x "${setup_probe_root}/mock-hermit"

cat > "${setup_probe_root}/mock-bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

case "$*" in
  info) exit 0 ;;
  "ps -a --format {{.Names}}") exit 0 ;;
  *) echo "unexpected docker invocation: $*" >&2; exit 1 ;;
esac
EOF
chmod +x "${setup_probe_root}/mock-bin/docker"

cat > "${setup_probe_root}/bin/just" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "$1" == "_ensure-services" ]]; then
  echo 'dev-setup reached Compose'
  exit 42
fi
echo "unexpected just invocation: $*" >&2
exit 1
EOF
chmod +x "${setup_probe_root}/bin/just"

setup_probe_output="${TMP_DIR}/setup-probe-output"
if (
  cd "${setup_probe_root}"
  PATH="${setup_probe_root}/mock-bin:/usr/bin:/bin" \
    HERMIT_EXE="${setup_probe_root}/mock-hermit" \
    ./scripts/dev-setup.sh
) > "${setup_probe_output}" 2>&1; then
  echo "dev-setup unexpectedly completed the no-system-node probe" >&2
  exit 1
fi
if ! grep -Fqx 'dev-setup reached Compose' "${setup_probe_output}"; then
  cat "${setup_probe_output}" >&2
  echo "dev-setup did not use its Hermit node shim before validating service URLs" >&2
  exit 1
fi

# A mismatched alternate loopback spelling must fail before setup starts
# Compose or migrations. The direct helper matrix above protects the real
# classifier; this probe protects its ordering in the setup boundary.
cat > "${setup_probe_root}/.env" <<'EOF'
PGPORT=15432
DATABASE_URL=postgres://buzz:do-not-print@localhost.:5432/buzz
EOF

setup_side_effects="${TMP_DIR}/setup-side-effects"
cat > "${setup_probe_root}/bin/just" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo compose >> "${SETUP_SIDE_EFFECTS}"
exit 1
EOF
chmod +x "${setup_probe_root}/bin/just"
cat > "${setup_probe_root}/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo migration >> "${SETUP_SIDE_EFFECTS}"
exit 1
EOF
chmod +x "${setup_probe_root}/bin/cargo"

if (
  cd "${setup_probe_root}"
  PATH="${setup_probe_root}/mock-bin:/usr/bin:/bin" \
    HERMIT_EXE="${setup_probe_root}/mock-hermit" \
    SETUP_SIDE_EFFECTS="${setup_side_effects}" \
    ./scripts/dev-setup.sh
) > "${setup_probe_output}" 2>&1; then
  echo "dev-setup unexpectedly accepted a mismatched local alias" >&2
  exit 1
fi
if ! grep -Fq 'DATABASE_URL URL uses local port 5432, but its Compose host port is 15432' "${setup_probe_output}"; then
  cat "${setup_probe_output}" >&2
  echo "dev-setup did not report the alternate-loopback port mismatch" >&2
  exit 1
fi
if [[ -e "${setup_side_effects}" ]]; then
  cat "${setup_side_effects}" >&2
  echo "dev-setup reached Compose or migrations after rejecting its service URL" >&2
  exit 1
fi
if grep -q 'do-not-print' "${setup_probe_output}"; then
  echo "dev-setup leaked URL credentials in its validation error" >&2
  exit 1
fi

echo "Dev service port checks passed"
