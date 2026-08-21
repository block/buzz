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
    const { BlockList, isIP } = require("node:net");

    try {
      const url = new URL(process.argv[1]);
      const rawHost = url.hostname.toLowerCase();
      const dnsHost = rawHost.endsWith(".") ? rawHost.slice(0, -1) : rawHost;

      // Reparse through a special-scheme URL to canonicalize legacy numeric
      // IPv4 forms such as 127.1, 0177.0.0.1, and 2130706433. The original
      // postgres:/redis: URLs are non-special and preserve those spellings.
      const canonicalHost = new URL(`http://${rawHost}/`)
        .hostname
        .replace(/^\[|\]$/g, "");
      const family = isIP(canonicalHost);
      const addressType = family === 4 ? "ipv4" : "ipv6";

      const loopback = new BlockList();
      loopback.addSubnet("127.0.0.0", 8, "ipv4");
      loopback.addAddress("::1", "ipv6");
      loopback.addSubnet("::ffff:127.0.0.0", 104, "ipv6");

      const unspecified = new BlockList();
      unspecified.addAddress("0.0.0.0", "ipv4");
      unspecified.addAddress("::", "ipv6");
      unspecified.addAddress("::ffff:0.0.0.0", "ipv6");

      const isLocalName = dnsHost === "localhost" || dnsHost.endsWith(".localhost");
      const isLoopbackAddress = family !== 0 && loopback.check(canonicalHost, addressType);
      const isUnspecifiedAddress = family !== 0 && unspecified.check(canonicalHost, addressType);
      const protocolDefault = url.protocol === "https:" ? "443"
        : url.protocol === "http:" ? "80"
        : process.argv[2];
      const port = url.port || protocolDefault;
      const location = isUnspecifiedAddress
        ? "unspecified"
        : isLocalName || isLoopbackAddress
          ? "local"
          : "remote";
      process.stdout.write(`${location}\t${port}`);
    } catch {
      process.exit(1);
    }
  ' "${url}" "${default_url_port}"); then
    echo "${service} URL is not a valid absolute URL" >&2
    return 1
  fi

  local location="${parsed%%$'\t'*}"
  local actual_port="${parsed#*$'\t'}"
  if [[ "${location}" == "unspecified" ]]; then
    echo "${service} URL uses an unspecified local address; use localhost, a loopback address, or an explicit remote host" >&2
    return 1
  fi
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
