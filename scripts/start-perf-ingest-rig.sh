#!/usr/bin/env bash
# =============================================================================
# start-perf-ingest-rig.sh — isolated two-community relay for the ingest-ceiling
# harness (perf/RELAY_INGEST_CEILING.md).
# =============================================================================
# Two communities on ONE relay process, resolved by Host: a.localhost and
# b.localhost both reach 127.0.0.1, so the URL host is the Host header and no
# proxy is needed. That is what lets the harness drive two communities at
# independent rates and tell the per-pod audit worker apart from the
# per-community audit lock.
#
# Reuses the `buzz-harness` Compose project and ports from
# docker-compose.harness.yml, so the shared :3000 dev stack is never touched.
#
# The relay's admission limits are raised deliberately. At defaults one identity
# is capped at 50 events per 5s, and a rejected EVENT gets a NOTICE with no OK,
# which stalls a NIP-01 client for its whole publish timeout. A sweep run at
# defaults measures the limiter, not the relay. perf/relay_ingest_ceiling.py
# invalidates any run where the quota-rejection metric moves.
#
# Emits the rig's coordinates as JSON on stdout; progress goes to stderr.
#
#   ./scripts/start-perf-ingest-rig.sh --reset            # first run
#   ./scripts/start-perf-ingest-rig.sh --audit off        # attribution control
#
# Teardown (the script verifies the pid is still this relay before signalling it;
# do the same by hand, since pids are recycled):
#   pid=$(cat /tmp/buzz-perf-ingest-rig.pid)
#   ps -p "$pid" -o command= | grep -qF "$PWD/target/ci/buzz-relay" && kill "$pid"
#   docker compose -p buzz-harness -f docker-compose.harness.yml down -v
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

PROJECT="buzz-harness"
COMPOSE_FILE="docker-compose.harness.yml"
PG_PORT=5471
REDIS_PORT=6471
MINIO_PORT=9471
RELAY_MAIN=3030
RELAY_HEALTH=8088
RELAY_METRICS=9202
HOST_A="a.localhost:${RELAY_MAIN}"
HOST_B="b.localhost:${RELAY_MAIN}"
PIDFILE=/tmp/buzz-perf-ingest-rig.pid
RELAY_LOG="${RELAY_LOG:-/tmp/buzz-perf-ingest-rig.log}"

# Far above any rate the harness offers, so the limiter cannot become the
# binding constraint. Both gates must be lifted: WsEvents is a 5s window,
# Messages a 60s one, so a short run only ever exercises the first.
WS_EVENTS_PER_SEC=100000
MESSAGES_PER_MIN=6000000

AUDIT=on
RESET=no
SKIP_RELAY=no
CARGO_PROFILE="${CARGO_PROFILE:-ci}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --audit) AUDIT="$2"; shift 2 ;;
    --reset) RESET=yes; shift ;;
    --skip-relay) SKIP_RELAY=yes; shift ;;
    --profile) CARGO_PROFILE="$2"; shift 2 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done
case "${AUDIT}" in on|off) ;; *) echo "--audit must be on or off" >&2; exit 1 ;; esac

case "${CARGO_PROFILE}" in
  dev|debug) CARGO_BUILD_PROFILE=dev; CARGO_TARGET_PROFILE=debug ;;
  *) CARGO_BUILD_PROFILE="${CARGO_PROFILE}"; CARGO_TARGET_PROFILE="${CARGO_PROFILE}" ;;
esac

log() { echo "[perf-rig] $*" >&2; }

psql_h() {
  docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" exec -T postgres \
    psql -U buzz -d buzz -v ON_ERROR_STOP=1 "$@"
}

log "Bringing up backing services (project=${PROJECT})..."
docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" up -d >&2

for _ in $(seq 1 60); do
  if psql_h -c 'SELECT 1' >/dev/null 2>&1; then break; fi
  sleep 2
done
psql_h -c 'SELECT 1' >/dev/null

if [[ "${RESET}" == yes ]]; then
  log "Resetting isolated database and applying schema..."
  psql_h -c 'DROP SCHEMA public CASCADE; CREATE SCHEMA public;' >/dev/null
  PGSCHEMA_PLAN_HOST=localhost PGSCHEMA_PLAN_PORT="${PG_PORT}" \
    PGSCHEMA_PLAN_DB=buzz PGSCHEMA_PLAN_USER=buzz PGSCHEMA_PLAN_PASSWORD=buzz_dev \
    PGHOST=localhost PGPORT="${PG_PORT}" PGUSER=buzz PGDATABASE=buzz PGPASSWORD=buzz_dev \
    ./bin/pgschema apply --file schema/schema.sql --auto-approve >&2
  psql_h < scripts/attach-schema-partitions.sql >/dev/null
fi

# The rustup shim honours rust-toolchain.toml; a stray Homebrew cargo does not.
if [[ -x "${HOME}/.cargo/bin/cargo" ]]; then
  export PATH="${HOME}/.cargo/bin:${PATH}"
fi
log "Building relay, CLI, and generator (profile=${CARGO_BUILD_PROFILE})..."
cargo build --profile "${CARGO_BUILD_PROFILE}" \
  -p buzz-relay -p buzz-cli -p buzz-test-client >&2

AUDIT_ENABLED=true
[[ "${AUDIT}" == off ]] && AUDIT_ENABLED=false

start_relay() {
  # Only signal a pid that is still our relay: pids are recycled, and a stale
  # file from a relay that already exited would otherwise kill a stranger.
  if [[ -f "${PIDFILE}" ]]; then
    stale_pid="$(cat "${PIDFILE}")"
    # The exact binary this rig launches, not any command containing
    # "buzz-relay": another checkout's relay must not be killed.
    if [[ "${stale_pid}" =~ ^[0-9]+$ ]] \
      && ps -p "${stale_pid}" -o command= 2>/dev/null \
        | grep -qF "${REPO_ROOT}/target/${CARGO_TARGET_PROFILE}/buzz-relay"; then
      kill "${stale_pid}" 2>/dev/null || true
    else
      log "pidfile ${PIDFILE} is stale (pid ${stale_pid} is not our relay); removing it"
    fi
    rm -f "${PIDFILE}"
  fi
  # The relay panics rather than reporting a conflict if the metrics port is taken,
  # so refuse to start instead of reporting somebody else's relay as this rig.
  for port in "${RELAY_MAIN}" "${RELAY_HEALTH}" "${RELAY_METRICS}"; do
    for _ in $(seq 1 15); do
      lsof -nP -iTCP:"${port}" -sTCP:LISTEN >/dev/null 2>&1 || break
      sleep 1
    done
    if lsof -nP -iTCP:"${port}" -sTCP:LISTEN >/dev/null 2>&1; then
      echo "[perf-rig] port ${port} is still in use; refusing to start" >&2
      exit 1
    fi
  done

  log "Starting relay on :${RELAY_MAIN} (audit_enabled=${AUDIT_ENABLED})..."
  # `env -u` scrubs any inherited CLI credentials: a stale BUZZ_AUTH_TAG fails the
  # local dev relay's first write outright.
  #
  # The relay is exec'd through `os.setsid()` because this script is normally
  # invoked from an ephemeral shell whose process group is reaped on return, which
  # SIGTERMs a plain background child seconds after the rig reports ready. A new
  # session detaches it. (The repo's other harness uses tmux for the same reason;
  # setsid needs nothing installed.)
  nohup env -u BUZZ_PRIVATE_KEY -u BUZZ_AUTH_TAG -u BUZZ_RELAY_URL \
    DATABASE_URL="postgres://buzz:buzz_dev@localhost:${PG_PORT}/buzz" \
    REDIS_URL="redis://localhost:${REDIS_PORT}" \
    RELAY_URL="ws://${HOST_A}" \
    BUZZ_BIND_ADDR="0.0.0.0:${RELAY_MAIN}" \
    BUZZ_HEALTH_PORT="${RELAY_HEALTH}" \
    BUZZ_METRICS_PORT="${RELAY_METRICS}" \
    BUZZ_S3_ENDPOINT="http://localhost:${MINIO_PORT}" \
    BUZZ_S3_ACCESS_KEY=buzz_dev \
    BUZZ_S3_SECRET_KEY=buzz_dev_secret \
    BUZZ_S3_BUCKET=buzz-media \
    BUZZ_REQUIRE_AUTH_TOKEN=false \
    BUZZ_AUDIT_ENABLED="${AUDIT_ENABLED}" \
    BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC="${WS_EVENTS_PER_SEC}" \
    BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN="${MESSAGES_PER_MIN}" \
    RUST_LOG=info \
    python3 -c 'import os, sys; os.setsid(); os.execv(sys.argv[1], sys.argv[1:])' \
    "${REPO_ROOT}/target/${CARGO_TARGET_PROFILE}/buzz-relay" > "${RELAY_LOG}" 2>&1 &
  echo $! > "${PIDFILE}"

  for _ in $(seq 1 60); do
    if curl -fs -o /dev/null "http://localhost:${RELAY_MAIN}/health"; then break; fi
    sleep 1
  done
  if ! curl -fs -o /dev/null "http://localhost:${RELAY_MAIN}/health"; then
    echo "[perf-rig] relay did not come up on :${RELAY_MAIN} — see ${RELAY_LOG}" >&2
    exit 1
  fi
}

if [[ "${SKIP_RELAY}" == yes ]]; then
  # Attach to a relay someone else is supervising — a debugger, a CI service
  # container, or an agent harness that reaps detached processes. The caller owns
  # matching --audit to how that relay was actually started; nothing here can
  # check it, so the audit-row control in perf/relay_ingest_ceiling.py is what
  # catches a mismatch.
  log "Attaching to the relay already listening on :${RELAY_MAIN}..."
  if ! curl -fs -o /dev/null "http://localhost:${RELAY_MAIN}/health"; then
    echo "[perf-rig] --skip-relay given but nothing is serving :${RELAY_MAIN}" >&2
    exit 1
  fi
else
  start_relay
fi

# Host A's community is seeded by the relay itself from RELAY_URL. Host B has no
# such hook, and the operator provisioning endpoint needs a NIP-98 signer the
# harness does not have, so insert the same row the startup path would.
psql_h -c "INSERT INTO communities (host) VALUES ('${HOST_B}') ON CONFLICT DO NOTHING;" >/dev/null

BENCH_KEY="$(openssl rand -hex 32)"
create_channel() {
  local host="$1" name="$2"
  env -u BUZZ_AUTH_TAG BUZZ_RELAY_URL="http://${host}" BUZZ_PRIVATE_KEY="${BENCH_KEY}" \
    "./target/${CARGO_TARGET_PROFILE}/buzz" \
    channels create --name "${name}" --type stream --visibility open \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["channel_id"])'
}
log "Creating a channel in each community..."
CHANNEL_A="$(create_channel "${HOST_A}" "perf-ingest-a")"
CHANNEL_B="$(create_channel "${HOST_B}" "perf-ingest-b")"

# Under --skip-relay this process manages no relay, so reporting a pid would
# hand the header's teardown instruction a number that is not ours to kill.
if [[ "${SKIP_RELAY}" == yes ]]; then
  RELAY_PID=null
else
  RELAY_PID="$(cat "${PIDFILE}")"
fi
SOURCE_REVISION="$(git -C "${REPO_ROOT}" rev-parse HEAD)"
# Two dirty trees at the same commit are two different builds, so the identity
# carries a digest of the working-tree diff rather than the commit alone.
SOURCE_DIFF_DIGEST="$(git -C "${REPO_ROOT}" diff HEAD | shasum -a 256 | cut -d' ' -f1)"
# The diff digest misses untracked inputs and is taken after the build, so it
# cannot prove the running binaries came from it. Hash the binaries themselves:
# that is the thing whose behaviour the dataset records.
BINARY_DIGEST="$(shasum -a 256 \
  "${REPO_ROOT}/target/${CARGO_TARGET_PROFILE}/buzz-relay" \
  "${REPO_ROOT}/target/${CARGO_TARGET_PROFILE}/ingest_load" \
  | shasum -a 256 | cut -d' ' -f1)"
DATABASE_RESET=false
[[ "${RESET}" == yes ]] && DATABASE_RESET=true
python3 -c '
import json, sys
(pid, log, gen, metrics, db, project, key, audit, ws_limit, msg_limit,
 host_a, chan_a, host_b, chan_b, revision, repo_root, diff_digest,
 database_reset, binary_digest) = sys.argv[1:]
print(json.dumps({
    "relay_pid": None if pid == "null" else int(pid),
    "source_revision": revision,
    "source_diff_digest": diff_digest,
    "binary_digest": binary_digest,
    "database_reset": database_reset == "true",
    "repo_root": repo_root,
    "relay_log": log,
    "generator": gen,
    "metrics_url": metrics,
    "database_url": db,
    "compose_project": project,
    "bench_private_key": key,
    "audit_enabled": audit == "true",
    "ws_events_per_sec_limit": int(ws_limit),
    "messages_per_min_limit": int(msg_limit),
    "targets": [
        {"community_host": host_a, "url": "ws://" + host_a, "channel": chan_a},
        {"community_host": host_b, "url": "ws://" + host_b, "channel": chan_b},
    ],
}, indent=2))
' "${RELAY_PID}" "${RELAY_LOG}" "./target/${CARGO_TARGET_PROFILE}/ingest_load" \
  "http://localhost:${RELAY_METRICS}/metrics" \
  "postgres://buzz:buzz_dev@localhost:${PG_PORT}/buzz" \
  "${PROJECT}" "${BENCH_KEY}" "${AUDIT_ENABLED}" \
  "${WS_EVENTS_PER_SEC}" "${MESSAGES_PER_MIN}" \
  "${HOST_A}" "${CHANNEL_A}" "${HOST_B}" "${CHANNEL_B}" \
  "${SOURCE_REVISION}" "${REPO_ROOT}" "${SOURCE_DIFF_DIGEST}" "${DATABASE_RESET}" \
  "${BINARY_DIGEST}"
log "Rig ready. Relay pid ${RELAY_PID}, log ${RELAY_LOG}, revision ${SOURCE_REVISION}"
