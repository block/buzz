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

pilot_relay_ready() {
  pilot_marker_matches "$relay_marker" "$relay_bin" \
    && [[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:3000/_readiness || true)" == '200' ]]
}

pilot_acp_ready() {
  local log="$PILOT_STATE_DIR/acp.log" pool_line connected_line discovered_line subscribed_line presence_line
  pilot_marker_matches "$acp_marker" "$acp_bin" || return 1
  grep -Fq "subscribed to channel ${PILOT_CHANNELS[CORE_SECOND_CHANNEL_ID]}" "$log" 2>/dev/null && return 1
  grep -Fq 'failed to subscribe' "$log" 2>/dev/null && return 1
  pool_line="$(grep -nF 'agent_pool_ready agents=1' "$log" 2>/dev/null | head -1 | cut -d: -f1)"
  connected_line="$(grep -nF "connected to relay at ${PILOT_ENV[BUZZ_RELAY_URL]}" "$log" 2>/dev/null | head -1 | cut -d: -f1)"
  discovered_line="$(grep -nE 'discovered ([2-9]|[1-9][0-9]+) channel\(s\)' "$log" 2>/dev/null | head -1 | cut -d: -f1)"
  subscribed_line="$(grep -nF "subscribed to channel ${PILOT_ENV[BUZZ_ACP_CHANNELS]}" "$log" 2>/dev/null | head -1 | cut -d: -f1)"
  presence_line="$(grep -nF 'presence set to online' "$log" 2>/dev/null | head -1 | cut -d: -f1)"
  [[ -n "$pool_line" && -n "$connected_line" && -n "$discovered_line" && -n "$subscribed_line" && -n "$presence_line" ]] \
    && (( pool_line < connected_line && connected_line < discovered_line \
      && discovered_line < subscribed_line && subscribed_line < presence_line ))
}

if pilot_relay_ready && pilot_acp_ready; then
  printf 'Core pilot is already running.\n'
  exit 0
fi

pilot_stop_marker "$relay_marker" "$relay_bin"
pilot_stop_marker "$acp_marker" "$acp_bin"

if command -v ss >/dev/null 2>&1; then
  listeners="$(ss -H -ltn 'sport = :3000' 2>/dev/null)" || { pilot_die 'unable to inspect relay port'; exit 1; }
  [[ -z "$listeners" ]] || { pilot_die 'relay port is occupied by a non-pilot process'; exit 1; }
elif (exec 3<>/dev/tcp/127.0.0.1/3000) 2>/dev/null; then
  pilot_die 'relay port is occupied by a non-pilot process'
  exit 1
fi

cd "$PILOT_REPO_ROOT"
docker compose up -d postgres redis minio minio-init

relay_log="$PILOT_STATE_DIR/relay.log"
acp_log="$PILOT_STATE_DIR/acp.log"
pilot_path="$PILOT_BIN_DIR:/usr/bin:/bin"

(
  trap '' HUP
  pilot_clear_environment
  export PATH="$pilot_path"
  export DATABASE_URL="${PILOT_ENV[DATABASE_URL]}"
  export REDIS_URL="${PILOT_ENV[REDIS_URL]}"
  export RELAY_URL="${PILOT_ENV[BUZZ_RELAY_URL]}"
  export BUZZ_BIND_ADDR="${PILOT_ENV[BUZZ_BIND_ADDR]}"
  export BUZZ_REQUIRE_AUTH_TOKEN="${PILOT_ENV[BUZZ_REQUIRE_AUTH_TOKEN]}"
  export BUZZ_REQUIRE_RELAY_MEMBERSHIP="${PILOT_ENV[BUZZ_REQUIRE_RELAY_MEMBERSHIP]}"
  export RELAY_OWNER_PUBKEY="${PILOT_ENV[CORE_BANKER_PUBLIC_KEY]}"
  export BUZZ_RELAY_PRIVATE_KEY="${PILOT_ENV[CORE_RELAY_PRIVATE_KEY]}"
  export BUZZ_GIT_ENABLED="${PILOT_ENV[BUZZ_GIT_ENABLED]}"
  exec "$relay_bin"
) </dev/null > "$relay_log" 2>&1 &
relay_pid=$!
marker_written=false
for _ in $(seq 1 10); do
  if pilot_write_marker "$relay_marker" "$relay_pid" "$relay_bin"; then
    marker_written=true
    break
  fi
  sleep 0.05
done
if [[ "$marker_written" != true ]]; then
  pilot_die 'relay exited before its ownership marker could be established'
  exit 1
fi

for _ in $(seq 1 30); do
  if ! pilot_marker_matches "$relay_marker" "$relay_bin"; then
    rm -f "$relay_marker"
    pilot_die 'relay exited during readiness'
    exit 1
  fi
  if pilot_relay_ready; then
    break
  fi
  sleep 1
done
if ! pilot_relay_ready; then
  pilot_stop_marker "$relay_marker" "$relay_bin"
  pilot_die 'relay did not become ready; see the pilot relay log'
  exit 1
fi

(
  trap '' HUP
  pilot_clear_environment
  export PATH="$pilot_path"
  export BUZZ_RELAY_URL="${PILOT_ENV[BUZZ_RELAY_URL]}"
  export RUST_LOG=info
  export BUZZ_PRIVATE_KEY="${PILOT_ENV[CORE_AGENT_PRIVATE_KEY]}"
  export OPENAI_COMPAT_API_KEY="${PILOT_ENV[OPENAI_COMPAT_API_KEY]}"
  export BUZZ_AGENT_PROVIDER="${PILOT_ENV[BUZZ_AGENT_PROVIDER]}"
  export BUZZ_AGENT_MODEL="${PILOT_ENV[BUZZ_AGENT_MODEL]}"
  export OPENAI_COMPAT_API="${PILOT_ENV[OPENAI_COMPAT_API]}"
  export OPENAI_COMPAT_BASE_URL="${PILOT_ENV[OPENAI_COMPAT_BASE_URL]}"
  export OPENAI_COMPAT_MODEL="${PILOT_ENV[OPENAI_COMPAT_MODEL]}"
  export BUZZ_AGENT_THINKING_EFFORT="${PILOT_ENV[BUZZ_AGENT_THINKING_EFFORT]}"
  export BUZZ_AGENT_WEB_SEARCH="${PILOT_ENV[BUZZ_AGENT_WEB_SEARCH]}"
  export BUZZ_AGENT_NO_HINTS="${PILOT_ENV[BUZZ_AGENT_NO_HINTS]}"
  export BUZZ_AGENT_REQUIRE_REPLY="${PILOT_ENV[BUZZ_AGENT_REQUIRE_REPLY]}"
  export BUZZ_ACP_SYSTEM_PROMPT_FILE="${PILOT_ENV[BUZZ_ACP_SYSTEM_PROMPT_FILE]}"
  export BUZZ_ACP_NO_BASE_PROMPT="${PILOT_ENV[BUZZ_ACP_NO_BASE_PROMPT]}"
  export BUZZ_ACP_NO_MEMORY="${PILOT_ENV[BUZZ_ACP_NO_MEMORY]}"
  export BUZZ_ACP_AGENT_COMMAND="${PILOT_ENV[BUZZ_ACP_AGENT_COMMAND]}"
  export BUZZ_ACP_AGENT_ARGS="${PILOT_ENV[BUZZ_ACP_AGENT_ARGS]}"
  export BUZZ_ACP_MODEL="${PILOT_ENV[BUZZ_ACP_MODEL]}"
  export BUZZ_ACP_MCP_COMMAND="${PILOT_ENV[BUZZ_ACP_MCP_COMMAND]}"
  export BUZZ_ACP_PUBLISH_AGENT_OUTPUT="${PILOT_ENV[BUZZ_ACP_PUBLISH_AGENT_OUTPUT]}"
  export BUZZ_ACP_AGENTS="${PILOT_ENV[BUZZ_ACP_AGENTS]}"
  export BUZZ_ACP_HEARTBEAT_INTERVAL="${PILOT_ENV[BUZZ_ACP_HEARTBEAT_INTERVAL]}"
  export BUZZ_ACP_SUBSCRIBE="${PILOT_ENV[BUZZ_ACP_SUBSCRIBE]}"
  export BUZZ_ACP_KINDS="${PILOT_ENV[BUZZ_ACP_KINDS]}"
  export BUZZ_ACP_CHANNELS="${PILOT_ENV[BUZZ_ACP_CHANNELS]}"
  export BUZZ_ACP_RESPOND_TO="${PILOT_ENV[BUZZ_ACP_RESPOND_TO]}"
  export BUZZ_ACP_AGENT_OWNER="${PILOT_ENV[BUZZ_ACP_AGENT_OWNER]}"
  export BUZZ_ACP_DEDUP="${PILOT_ENV[BUZZ_ACP_DEDUP]}"
  export BUZZ_ACP_MULTIPLE_EVENT_HANDLING="${PILOT_ENV[BUZZ_ACP_MULTIPLE_EVENT_HANDLING]}"
  exec "$acp_bin"
) </dev/null > "$acp_log" 2>&1 &
acp_pid=$!
marker_written=false
for _ in $(seq 1 10); do
  if pilot_write_marker "$acp_marker" "$acp_pid" "$acp_bin"; then
    marker_written=true
    break
  fi
  sleep 0.05
done
if [[ "$marker_written" != true ]]; then
  pilot_stop_marker "$relay_marker" "$relay_bin"
  pilot_die 'ACP exited before its ownership marker could be established'
  exit 1
fi

for _ in $(seq 1 30); do
  if ! pilot_marker_matches "$acp_marker" "$acp_bin"; then
    rm -f "$acp_marker"
    pilot_stop_marker "$relay_marker" "$relay_bin"
    pilot_die 'ACP exited before connection and channel subscription readiness'
    exit 1
  fi
  if grep -Fq "subscribed to channel ${PILOT_CHANNELS[CORE_SECOND_CHANNEL_ID]}" "$acp_log" 2>/dev/null \
    || grep -Fq 'failed to subscribe' "$acp_log" 2>/dev/null; then
    pilot_stop_marker "$acp_marker" "$acp_bin"
    pilot_stop_marker "$relay_marker" "$relay_bin"
    pilot_die 'ACP reported an unsafe or failed channel subscription'
    exit 1
  fi
  if pilot_acp_ready; then
    break
  fi
  sleep 1
done
if ! pilot_acp_ready || ! pilot_relay_ready; then
  pilot_stop_marker "$acp_marker" "$acp_bin"
  pilot_stop_marker "$relay_marker" "$relay_bin"
  pilot_die 'pilot stack did not reach connected subscription readiness'
  exit 1
fi

printf 'Core pilot is ready at ws://127.0.0.1:3000.\n'
