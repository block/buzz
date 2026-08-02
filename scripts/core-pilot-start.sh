#!/usr/bin/env bash
# Start the local-only Core pilot without builds, installs, Docker resets, or Desktop launch.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PILOT_REPO_ROOT="$(cd "$script_dir/.." && pwd)"
# This is a repository-owned helper, not a user configuration file.
source "$script_dir/core-pilot-lib.sh"

pilot_parse_paths "$@"
pilot_load_and_validate
command -v docker >/dev/null 2>&1 || { pilot_die 'Docker is required to start the pilot'; exit 1; }

relay_marker="$PILOT_STATE_DIR/relay.pid"
acp_marker="$PILOT_STATE_DIR/acp.pid"
relay_bin="$PILOT_BIN_DIR/buzz-relay"
acp_bin="$PILOT_BIN_DIR/buzz-acp"

if pilot_marker_matches "$relay_marker" "$relay_bin" && pilot_marker_matches "$acp_marker" "$acp_bin"; then
  printf 'Core pilot is already running.\n'
  exit 0
fi

pilot_stop_marker "$relay_marker" "$relay_bin"
pilot_stop_marker "$acp_marker" "$acp_bin"

cd "$PILOT_REPO_ROOT"
docker compose up -d postgres redis minio minio-init

relay_log="$PILOT_STATE_DIR/relay.log"
acp_log="$PILOT_STATE_DIR/acp.log"
pilot_path="$PILOT_BIN_DIR:$PATH"

nohup env -i \
  "PATH=$pilot_path" \
  "DATABASE_URL=${PILOT_ENV[DATABASE_URL]}" \
  "REDIS_URL=${PILOT_ENV[REDIS_URL]}" \
  "RELAY_URL=${PILOT_ENV[BUZZ_RELAY_URL]}" \
  "BUZZ_BIND_ADDR=${PILOT_ENV[BUZZ_BIND_ADDR]}" \
  "BUZZ_REQUIRE_AUTH_TOKEN=${PILOT_ENV[BUZZ_REQUIRE_AUTH_TOKEN]}" \
  "BUZZ_GIT_ENABLED=${PILOT_ENV[BUZZ_GIT_ENABLED]}" \
  "$relay_bin" > "$relay_log" 2>&1 &
printf '%s|%s\n' "$!" "$relay_bin" > "$relay_marker"

for _ in $(seq 1 30); do
  if [[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:3000/_readiness || true)" == '200' ]]; then
    break
  fi
  sleep 1
done
if [[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:3000/_readiness || true)" != '200' ]]; then
  pilot_stop_marker "$relay_marker" "$relay_bin"
  pilot_die 'relay did not become ready; see the pilot relay log'
  exit 1
fi

nohup env -i \
  "PATH=$pilot_path" \
  "BUZZ_RELAY_URL=${PILOT_ENV[BUZZ_RELAY_URL]}" \
  "BUZZ_PRIVATE_KEY=${PILOT_ENV[BUZZ_PRIVATE_KEY]}" \
  "OPENAI_COMPAT_API_KEY=${PILOT_ENV[OPENAI_COMPAT_API_KEY]}" \
  "BUZZ_AGENT_PROVIDER=${PILOT_ENV[BUZZ_AGENT_PROVIDER]}" \
  "OPENAI_COMPAT_API=${PILOT_ENV[OPENAI_COMPAT_API]}" \
  "OPENAI_COMPAT_BASE_URL=${PILOT_ENV[OPENAI_COMPAT_BASE_URL]}" \
  "OPENAI_COMPAT_MODEL=${PILOT_ENV[OPENAI_COMPAT_MODEL]}" \
  "BUZZ_AGENT_THINKING_EFFORT=${PILOT_ENV[BUZZ_AGENT_THINKING_EFFORT]}" \
  "BUZZ_AGENT_WEB_SEARCH=${PILOT_ENV[BUZZ_AGENT_WEB_SEARCH]}" \
  "BUZZ_AGENT_NO_HINTS=${PILOT_ENV[BUZZ_AGENT_NO_HINTS]}" \
  "BUZZ_AGENT_REQUIRE_REPLY=${PILOT_ENV[BUZZ_AGENT_REQUIRE_REPLY]}" \
  "BUZZ_ACP_SYSTEM_PROMPT_FILE=${PILOT_ENV[BUZZ_ACP_SYSTEM_PROMPT_FILE]}" \
  "BUZZ_ACP_NO_BASE_PROMPT=${PILOT_ENV[BUZZ_ACP_NO_BASE_PROMPT]}" \
  "BUZZ_ACP_NO_MEMORY=${PILOT_ENV[BUZZ_ACP_NO_MEMORY]}" \
  "BUZZ_ACP_AGENT_COMMAND=${PILOT_ENV[BUZZ_ACP_AGENT_COMMAND]}" \
  "BUZZ_ACP_AGENT_ARGS=${PILOT_ENV[BUZZ_ACP_AGENT_ARGS]}" \
  "BUZZ_ACP_MCP_COMMAND=${PILOT_ENV[BUZZ_ACP_MCP_COMMAND]}" \
  "BUZZ_ACP_PUBLISH_AGENT_OUTPUT=${PILOT_ENV[BUZZ_ACP_PUBLISH_AGENT_OUTPUT]}" \
  "BUZZ_ACP_AGENTS=${PILOT_ENV[BUZZ_ACP_AGENTS]}" \
  "BUZZ_ACP_HEARTBEAT_INTERVAL=${PILOT_ENV[BUZZ_ACP_HEARTBEAT_INTERVAL]}" \
  "BUZZ_ACP_SUBSCRIBE=${PILOT_ENV[BUZZ_ACP_SUBSCRIBE]}" \
  "BUZZ_ACP_KINDS=${PILOT_ENV[BUZZ_ACP_KINDS]}" \
  "BUZZ_ACP_CHANNELS=${PILOT_ENV[BUZZ_ACP_CHANNELS]}" \
  "BUZZ_ACP_RESPOND_TO=${PILOT_ENV[BUZZ_ACP_RESPOND_TO]}" \
  "BUZZ_ACP_AGENT_OWNER=${PILOT_ENV[BUZZ_ACP_AGENT_OWNER]}" \
  "BUZZ_ACP_DEDUP=${PILOT_ENV[BUZZ_ACP_DEDUP]}" \
  "BUZZ_ACP_MULTIPLE_EVENT_HANDLING=${PILOT_ENV[BUZZ_ACP_MULTIPLE_EVENT_HANDLING]}" \
  "$acp_bin" > "$acp_log" 2>&1 &
printf '%s|%s\n' "$!" "$acp_bin" > "$acp_marker"

printf 'Core pilot is ready at ws://127.0.0.1:3000.\n'
