#!/usr/bin/env bash
# Shared validation helpers for root development service configuration.

validate_local_service_port() {
  local service="$1"
  local url="$2"
  local expected_port="$3"
  local default_url_port="$4"
  local parsed

  # shellcheck disable=SC2016
  if ! parsed=$(node -e '
    try {
      const url = new URL(process.argv[1]);
      const host = url.hostname.toLowerCase();
      const isLocal = host === "localhost" || host === "127.0.0.1" || host === "[::1]";
      const port = url.port || process.argv[2];
      process.stdout.write(`${isLocal ? "local" : "remote"}\t${port}`);
    } catch {
      process.exit(1);
    }
  ' "${url}" "${default_url_port}"); then
    echo "${service} URL is not a valid absolute URL" >&2
    return 1
  fi

  local location="${parsed%%$'\t'*}"
  local actual_port="${parsed#*$'\t'}"
  if [[ "${location}" == "local" && "${actual_port}" != "${expected_port}" ]]; then
    echo "${service} URL uses local port ${actual_port}, but its Compose host port is ${expected_port}" >&2
    return 1
  fi
}

compose_service_host_port() {
  local service="$1"
  local container_port="$2"
  local binding
  local host_port

  if ! binding=$(docker compose port "${service}" "${container_port}" 2>/dev/null); then
    echo "Could not resolve the published port for ${service}:${container_port}" >&2
    return 1
  fi
  binding="${binding##*$'\n'}"
  host_port="${binding##*:}"
  if [[ ! "${host_port}" =~ ^[0-9]+$ ]]; then
    echo "Invalid published port for ${service}:${container_port}" >&2
    return 1
  fi

  printf '%s\n' "${host_port}"
}

# Configure the shared relay-test launcher from the ports Compose actually
# published. Service hosts and credentials stay pinned to the local dev stack,
# so a developer's remote DATABASE_URL/PG* environment can never become an
# auto-approved schema target.
configure_local_compose_service_env() {
  local postgres_port
  local redis_port
  local minio_port

  postgres_port=$(compose_service_host_port postgres 5432)
  redis_port=$(compose_service_host_port redis 6379)
  minio_port=$(compose_service_host_port minio 9000)

  export PGHOST=localhost
  export PGPORT="${postgres_port}"
  export PGUSER=buzz
  export PGPASSWORD=buzz_dev
  export PGDATABASE=buzz
  export DATABASE_URL="postgres://buzz:buzz_dev@localhost:${PGPORT}/buzz"

  export PGSCHEMA_PLAN_HOST=localhost
  export PGSCHEMA_PLAN_PORT="${PGPORT}"
  export PGSCHEMA_PLAN_DB=buzz
  export PGSCHEMA_PLAN_USER=buzz
  export PGSCHEMA_PLAN_PASSWORD=buzz_dev

  export REDIS_PORT="${redis_port}"
  export REDIS_URL="redis://localhost:${REDIS_PORT}"

  export MINIO_API_PORT="${minio_port}"
  export BUZZ_S3_ENDPOINT="http://localhost:${MINIO_API_PORT}"
  export BUZZ_S3_ACCESS_KEY=buzz_dev
  export BUZZ_S3_SECRET_KEY=buzz_dev_secret
  export BUZZ_S3_BUCKET=buzz-media
}
