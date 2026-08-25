#!/usr/bin/env bash
set -euo pipefail

health_port="${BUZZ_HEALTH_PORT:-8080}"
deadline=$((SECONDS + 60))
relay_pid=""
relay_log="${BUZZ_V32_RELAY_LOG:-/tmp/buzz-v32-macos-relay.log}"
auto_stack=0

if ! lsof -nP -iTCP:3000 -sTCP:LISTEN >/dev/null 2>&1; then
  if [[ -z "${DATABASE_URL:-}" || -z "${REDIS_URL:-}" || -z "${BUZZ_S3_ENDPOINT:-}" ]]; then
    if ! command -v docker >/dev/null 2>&1; then
      echo "Docker is required to start the isolated Relay backing services."
      exit 1
    fi
    compose=(docker compose -p buzz-v32-macos -f docker-compose.harness.yml)
    "${compose[@]}" up -d --wait postgres redis minio
    "${compose[@]}" run --rm minio-init
    auto_stack=1
  fi

  export DATABASE_URL="${DATABASE_URL:-postgres://buzz:buzz_dev@127.0.0.1:5471/buzz}"
  export REDIS_URL="${REDIS_URL:-redis://127.0.0.1:6471}"
  export BUZZ_S3_ENDPOINT="${BUZZ_S3_ENDPOINT:-http://127.0.0.1:9471}"
  export BUZZ_S3_ACCESS_KEY="${BUZZ_S3_ACCESS_KEY:-buzz_dev}"
  export BUZZ_S3_SECRET_KEY="${BUZZ_S3_SECRET_KEY:-buzz_dev_secret}"
  export BUZZ_S3_BUCKET="${BUZZ_S3_BUCKET:-buzz-media}"
  export BUZZ_S3_REGION="${BUZZ_S3_REGION:-us-east-1}"
  export BUZZ_S3_ADDRESSING_STYLE="${BUZZ_S3_ADDRESSING_STYLE:-path}"

  admin_binary=""
  for candidate in target/release/buzz-admin target/debug/buzz-admin; do
    if [[ -x "$candidate" ]]; then admin_binary="$candidate"; break; fi
  done
  if [[ -z "$admin_binary" ]]; then
    echo "Migration binary is missing. Build it first with: cargo build --release -p buzz-admin"
    exit 1
  fi
  "$admin_binary" migrate

  # The Relay seeds RELAY_URL's primary host, while clean Desktop onboarding
  # may use either loopback spelling. Row-zero host binding intentionally treats
  # those authorities as distinct, so seed only the isolated harness aliases.
  if (( auto_stack == 1 )); then
    "${compose[@]}" exec -T postgres \
      psql -U buzz -d buzz -v ON_ERROR_STOP=1 -c \
      "INSERT INTO communities (host) VALUES
         ('localhost'), ('127.0.0.1'), ('localhost:3000'), ('127.0.0.1:3000')
       ON CONFLICT (lower(host)) DO NOTHING;"
  fi

  relay_binary=""
  for candidate in target/release/buzz-relay target/debug/buzz-relay; do
    if [[ -x "$candidate" ]]; then relay_binary="$candidate"; break; fi
  done
  if [[ -z "$relay_binary" ]]; then
    echo "Relay binary is missing. Build it first with: cargo build --release -p buzz-relay"
    exit 1
  fi
  "$relay_binary" >"$relay_log" 2>&1 &
  relay_pid=$!
  trap '[[ -n "$relay_pid" ]] && kill "$relay_pid" 2>/dev/null || true' EXIT
fi
until curl --fail --silent --max-time 2 "http://127.0.0.1:${health_port}/_readiness" >/dev/null 2>&1; do
  if [[ -n "$relay_pid" ]] && ! kill -0 "$relay_pid" 2>/dev/null; then
    echo "Relay exited before readiness"
    tail -n 80 "$relay_log"
    exit 1
  fi
  if (( SECONDS >= deadline )); then
    echo "Relay readiness timed out after 60 seconds"
    exit 1
  fi
  sleep 1
done

app="desktop/src-tauri/target/release/bundle/macos/Buzz Dev.app"
if [[ ! -d "$app" ]]; then
  echo "Isolated Debug app is missing: $app"
  echo "Build with the V3.2 documented tauri command before acceptance."
  exit 1
fi
echo "Relay ready; launching isolated Buzz Dev acceptance bundle. Relay log: $relay_log"
open -W "$app"
