#!/usr/bin/env bash
# Print secret-safe state and recent logs for Compose dependencies.
set -u

services=("$@")
if [[ "${#services[@]}" -eq 0 ]]; then
  services=(postgres redis rustfs rustfs-init)
fi

echo "::group::Buzz dependency container state"
docker compose ps -a "${services[@]}" 2>&1 || true
echo "::endgroup::"

echo "::group::Buzz dependency logs (last 200 lines)"
docker compose logs --no-color --tail=200 "${services[@]}" 2>&1 || true
echo "::endgroup::"
