#!/usr/bin/env bash
# wait-healthy.sh — Poll a Docker container's health status until healthy.
# Usage: ./scripts/wait-healthy.sh <service-label> <container-name> [timeout-seconds]
set -euo pipefail

SERVICE="${1:?usage: wait-healthy.sh <service> <container> [timeout]}"
CONTAINER="${2:?usage: wait-healthy.sh <service> <container> [timeout]}"
TIMEOUT="${3:-120}"

INTERVAL=2
ATTEMPTS=$(( TIMEOUT / INTERVAL ))

for attempt in $(seq 1 "${ATTEMPTS}"); do
  status=$(docker inspect --format='{{.State.Health.Status}}' "${CONTAINER}" 2>/dev/null || echo "not_found")
  if [ "${status}" = "healthy" ]; then
    echo "${SERVICE} is healthy"
    exit 0
  fi
  sleep "${INTERVAL}"
done

echo "${SERVICE} did not become healthy within ${TIMEOUT}s" >&2
docker logs "${CONTAINER}" || true
exit 1
