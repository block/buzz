#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK_DIR="$(mktemp -d /tmp/buzz-compose-test.XXXXXXXXXX)"

cleanup() {
  case "${WORK_DIR}" in
    /tmp/buzz-compose-test.*) rm -rf -- "${WORK_DIR}" ;;
  esac
}
trap cleanup EXIT

install -d "${WORK_DIR}/deploy/compose"
cp \
  "${SCRIPT_DIR}/.env.example" \
  "${SCRIPT_DIR}/Caddyfile" \
  "${SCRIPT_DIR}/compose.yml" \
  "${SCRIPT_DIR}/compose.caddy.yml" \
  "${SCRIPT_DIR}/compose.dev.yml" \
  "${WORK_DIR}/deploy/compose/"
cp "${SCRIPT_DIR}/../../prometheus.yml" "${WORK_DIR}/prometheus.yml"
cp "${SCRIPT_DIR}/.env.example" "${WORK_DIR}/deploy/compose/.env"

cd "${WORK_DIR}/deploy/compose"

render() {
  local name="$1"
  shift
  docker compose \
    --env-file .env \
    -f compose.yml \
    "$@" \
    config --quiet
  printf 'Validated production Compose render: %s\n' "${name}"
}

render base
render tls -f compose.caddy.yml
render dev -f compose.dev.yml
render tls-dev -f compose.caddy.yml -f compose.dev.yml
