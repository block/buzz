#!/usr/bin/env bash
# Read-only smoke check for Steve's local Buzz pilot community.
set -euo pipefail

RELAY_HTTP_URL="${BUZZ_PILOT_RELAY_HTTP_URL:-http://localhost:3030}"
HEALTH_URL="${BUZZ_PILOT_HEALTH_URL:-http://127.0.0.1:8088/_readiness}"
CHANNEL_ID="${BUZZ_PILOT_CHANNEL_ID:-3cdf4550-0501-4825-b54e-87213ea08b66}"
SUMMARY_EVENT_ID="${BUZZ_PILOT_SUMMARY_EVENT_ID:-295d3891fb6a200a325f148ed651e4fc519f7b51f9d15bb9cad84b041871d8aa}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
HELPER_ROOT="$(cd "${REPO_ROOT}/.." && pwd)"
BUZZ_CLI="${BUZZ_PILOT_CLI:-${HELPER_ROOT}/scripts/buzz}"
PRIVATE_KEY="${BUZZ_PRIVATE_KEY:-}"

if [[ -z "${PRIVATE_KEY}" ]]; then
  if command -v openssl >/dev/null 2>&1; then
    PRIVATE_KEY="$(openssl rand -hex 32)"
  else
    echo "error: BUZZ_PRIVATE_KEY is unset and openssl is unavailable for a disposable read key" >&2
    exit 2
  fi
fi

if [[ ! -x "${BUZZ_CLI}" ]]; then
  echo "error: Buzz CLI not executable at ${BUZZ_CLI}" >&2
  echo "Set BUZZ_PILOT_CLI to the project-local helper path or build the CLI first." >&2
  exit 2
fi

echo "Checking Steve Buzz pilot relay health: ${HEALTH_URL}"
if ! curl --silent --show-error --fail "${HEALTH_URL}" >/dev/null; then
  cat >&2 <<EOF
error: active Buzz pilot relay is not ready.

Start it from the upstream checkout with:
  RELAY_URL=ws://localhost:3030 \\
  BUZZ_BIND_ADDR=127.0.0.1:3030 \\
  BUZZ_HEALTH_PORT=8088 \\
  BUZZ_METRICS_PORT=9202 \\
  BUZZ_RELAY_URL=ws://localhost:3030 \\
  just relay
EOF
  exit 1
fi

echo "Listing active Day 0 channels through ${RELAY_HTTP_URL}"
channels_json="$(
  BUZZ_RELAY_URL="${RELAY_HTTP_URL}" \
  BUZZ_PRIVATE_KEY="${PRIVATE_KEY}" \
  "${BUZZ_CLI}" --format compact channels list
)"

if ! printf '%s\n' "${channels_json}" | grep -Fq '"name":"buzz-pilot"'; then
  echo "error: buzz-pilot channel was not visible through ${RELAY_HTTP_URL}" >&2
  printf '%s\n' "${channels_json}" >&2
  exit 1
fi

echo "Reading bounded buzz-pilot messages"
messages_json="$(
  BUZZ_RELAY_URL="${RELAY_HTTP_URL}" \
  BUZZ_PRIVATE_KEY="${PRIVATE_KEY}" \
  "${BUZZ_CLI}" --format compact messages get \
    --channel "${CHANNEL_ID}" \
    --limit 10
)"

if ! printf '%s\n' "${messages_json}" | grep -Fq "${SUMMARY_EVENT_ID}"; then
  echo "error: archive summary event was not found in recent buzz-pilot messages" >&2
  echo "Expected event: ${SUMMARY_EVENT_ID}" >&2
  printf '%s\n' "${messages_json}" >&2
  exit 1
fi

echo "ok: active Buzz pilot is ready on ${RELAY_HTTP_URL}; archive summary event is visible."
