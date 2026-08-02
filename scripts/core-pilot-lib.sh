#!/usr/bin/env bash
# Shared, deliberately narrow configuration handling for the Core local pilot.

pilot_die() {
  printf 'core-pilot: %s\n' "$*" >&2
  return 1
}

pilot_default_config_file() {
  printf '%s/core-buzz/pilot.env' "${XDG_CONFIG_HOME:-"$HOME/.config"}"
}

pilot_default_secrets_file() {
  printf '%s/core-buzz/agent.env' "${XDG_CONFIG_HOME:-"$HOME/.config"}"
}

pilot_default_state_dir() {
  printf '%s/core-buzz' "${XDG_STATE_HOME:-"$HOME/.local/state"}"
}

pilot_parse_paths() {
  PILOT_CONFIG_FILE="$(pilot_default_config_file)"
  PILOT_SECRETS_FILE="$(pilot_default_secrets_file)"
  PILOT_STATE_DIR="$(pilot_default_state_dir)"

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --config)
        [[ $# -ge 2 ]] || pilot_die '--config requires a file' || return 1
        PILOT_CONFIG_FILE="$2"
        shift 2
        ;;
      --secrets)
        [[ $# -ge 2 ]] || pilot_die '--secrets requires a file' || return 1
        PILOT_SECRETS_FILE="$2"
        shift 2
        ;;
      --state-dir)
        [[ $# -ge 2 ]] || pilot_die '--state-dir requires a directory' || return 1
        PILOT_STATE_DIR="$2"
        shift 2
        ;;
      *)
        pilot_die "unknown option: $1" || return 1
        ;;
    esac
  done
}

pilot_config_key_allowed() {
  case "$1" in
    BUZZ_RELAY_URL|BUZZ_BIND_ADDR|DATABASE_URL|REDIS_URL|BUZZ_REQUIRE_AUTH_TOKEN|BUZZ_GIT_ENABLED|\
    BUZZ_AGENT_PROVIDER|OPENAI_COMPAT_API|OPENAI_COMPAT_BASE_URL|OPENAI_COMPAT_MODEL|\
    BUZZ_AGENT_THINKING_EFFORT|BUZZ_AGENT_WEB_SEARCH|BUZZ_AGENT_NO_HINTS|BUZZ_AGENT_REQUIRE_REPLY|\
    BUZZ_ACP_SYSTEM_PROMPT_FILE|BUZZ_ACP_NO_BASE_PROMPT|BUZZ_ACP_NO_MEMORY|BUZZ_ACP_AGENT_COMMAND|\
    BUZZ_ACP_AGENT_ARGS|BUZZ_ACP_MCP_COMMAND|BUZZ_ACP_PUBLISH_AGENT_OUTPUT|BUZZ_ACP_AGENTS|\
    BUZZ_ACP_HEARTBEAT_INTERVAL|BUZZ_ACP_SUBSCRIBE|BUZZ_ACP_KINDS|BUZZ_ACP_CHANNELS|\
    BUZZ_ACP_RESPOND_TO|BUZZ_ACP_AGENT_OWNER|BUZZ_ACP_DEDUP|BUZZ_ACP_MULTIPLE_EVENT_HANDLING)
      return 0
      ;;
  esac
  return 1
}

pilot_secret_key_allowed() {
  [[ "$1" == 'OPENAI_COMPAT_API_KEY' || "$1" == 'BUZZ_PRIVATE_KEY' ]]
}

pilot_is_placeholder() {
  local lowered="${1,,}"
  [[ "$lowered" == *placeholder* || "$lowered" == *replace* || "$lowered" == *change_me* || \
     "$lowered" == *changeme* || "$lowered" == *your_* || "$lowered" == *example* || "$lowered" == *dummy* ]]
}

pilot_read_file() {
  local file="$1" kind="$2" line key value
  [[ -f "$file" ]] || pilot_die "$kind file is missing" || return 1
  [[ -r "$file" ]] || pilot_die "$kind file is not readable" || return 1

  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    [[ -z "$line" || "$line" == \#* ]] && continue
    if [[ ! "$line" =~ ^([A-Z][A-Z0-9_]*)=(.*)$ ]]; then
      pilot_die "$kind file contains a malformed record" || return 1
    fi
    key="${BASH_REMATCH[1]}"
    value="${BASH_REMATCH[2]}"
    if [[ "$kind" == 'configuration' ]]; then
      pilot_config_key_allowed "$key" || { pilot_die "configuration contains an unsupported setting"; return 1; }
    else
      pilot_secret_key_allowed "$key" || { pilot_die "secret file contains an unsupported setting"; return 1; }
    fi
    [[ -z "${PILOT_ENV[$key]+set}" ]] || { pilot_die "$kind file contains a duplicate setting"; return 1; }
    if [[ "$key" != 'BUZZ_ACP_MCP_COMMAND' && -z "$value" ]]; then
      pilot_die "$kind file contains an empty required value" || return 1
    fi
    [[ "$value" != *$'\n'* && "$value" != *$'\r'* && "$value" != *[[:space:]]* ]] || {
      pilot_die "$kind file contains an unsafe value"; return 1;
    }
    if [[ "$kind" == 'secret' ]] && pilot_is_placeholder "$value"; then
      pilot_die 'secret file contains a placeholder value' || return 1
    fi
    PILOT_ENV["$key"]="$value"
  done < "$file"
}

pilot_require() {
  [[ -n "${PILOT_ENV[$1]+set}" ]] || { pilot_die "required pilot setting is missing"; return 1; }
}

pilot_require_value() {
  pilot_require "$1" || return 1
  [[ "${PILOT_ENV[$1]}" == "$2" ]] || { pilot_die "pilot setting is not approved"; return 1; }
}

pilot_validate_config() {
  local required key prompt channels owner normalized_url
  required=(
    BUZZ_RELAY_URL BUZZ_BIND_ADDR DATABASE_URL REDIS_URL BUZZ_REQUIRE_AUTH_TOKEN BUZZ_GIT_ENABLED
    BUZZ_AGENT_PROVIDER OPENAI_COMPAT_API OPENAI_COMPAT_BASE_URL OPENAI_COMPAT_MODEL
    BUZZ_AGENT_THINKING_EFFORT BUZZ_AGENT_WEB_SEARCH BUZZ_AGENT_NO_HINTS BUZZ_AGENT_REQUIRE_REPLY
    BUZZ_ACP_SYSTEM_PROMPT_FILE BUZZ_ACP_NO_BASE_PROMPT BUZZ_ACP_NO_MEMORY BUZZ_ACP_AGENT_COMMAND
    BUZZ_ACP_AGENT_ARGS BUZZ_ACP_MCP_COMMAND BUZZ_ACP_PUBLISH_AGENT_OUTPUT BUZZ_ACP_AGENTS
    BUZZ_ACP_HEARTBEAT_INTERVAL BUZZ_ACP_SUBSCRIBE BUZZ_ACP_KINDS BUZZ_ACP_CHANNELS
    BUZZ_ACP_RESPOND_TO BUZZ_ACP_AGENT_OWNER BUZZ_ACP_DEDUP BUZZ_ACP_MULTIPLE_EVENT_HANDLING
    OPENAI_COMPAT_API_KEY BUZZ_PRIVATE_KEY
  )
  for key in "${required[@]}"; do
    pilot_require "$key" || return 1
  done

  pilot_require_value BUZZ_RELAY_URL 'ws://127.0.0.1:3000' || return 1
  pilot_require_value BUZZ_BIND_ADDR '127.0.0.1:3000' || return 1
  pilot_require_value DATABASE_URL 'postgres://buzz:buzz_dev@127.0.0.1:5432/buzz' || return 1
  pilot_require_value REDIS_URL 'redis://127.0.0.1:6379' || return 1
  pilot_require_value BUZZ_REQUIRE_AUTH_TOKEN false || return 1
  pilot_require_value BUZZ_GIT_ENABLED false || return 1
  pilot_require_value BUZZ_AGENT_PROVIDER openai || return 1
  pilot_require_value OPENAI_COMPAT_API responses || return 1
  normalized_url="${PILOT_ENV[OPENAI_COMPAT_BASE_URL]%/}"
  [[ "$normalized_url" == 'https://api.openai.com/v1' ]] || { pilot_die 'OpenAI URL is not canonical'; return 1; }
  pilot_require_value OPENAI_COMPAT_MODEL gpt-5.6-terra || return 1
  pilot_require_value BUZZ_AGENT_THINKING_EFFORT medium || return 1
  pilot_require_value BUZZ_AGENT_WEB_SEARCH 1 || return 1
  pilot_require_value BUZZ_AGENT_NO_HINTS 1 || return 1
  pilot_require_value BUZZ_AGENT_REQUIRE_REPLY 0 || return 1
  pilot_require_value BUZZ_ACP_NO_BASE_PROMPT 1 || return 1
  pilot_require_value BUZZ_ACP_NO_MEMORY 1 || return 1
  pilot_require_value BUZZ_ACP_AGENT_COMMAND buzz-agent || return 1
  pilot_require_value BUZZ_ACP_AGENT_ARGS acp || return 1
  pilot_require_value BUZZ_ACP_MCP_COMMAND '' || return 1
  pilot_require_value BUZZ_ACP_PUBLISH_AGENT_OUTPUT trigger-reply || return 1
  pilot_require_value BUZZ_ACP_AGENTS 1 || return 1
  pilot_require_value BUZZ_ACP_HEARTBEAT_INTERVAL 0 || return 1
  pilot_require_value BUZZ_ACP_SUBSCRIBE all || return 1
  pilot_require_value BUZZ_ACP_KINDS 9 || return 1
  pilot_require_value BUZZ_ACP_RESPOND_TO owner-only || return 1
  pilot_require_value BUZZ_ACP_DEDUP queue || return 1
  pilot_require_value BUZZ_ACP_MULTIPLE_EVENT_HANDLING queue || return 1

  channels="${PILOT_ENV[BUZZ_ACP_CHANNELS]}"
  [[ "$channels" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-4[0-9a-fA-F]{3}-[89aAbB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}$ ]] || {
    pilot_die 'pilot requires exactly one UUID channel'; return 1;
  }
  owner="${PILOT_ENV[BUZZ_ACP_AGENT_OWNER]}"
  [[ "$owner" =~ ^[0-9a-fA-F]{64}$ ]] || { pilot_die 'pilot owner must be a public key'; return 1; }
  [[ -n "${PILOT_ENV[OPENAI_COMPAT_API_KEY]}" && -n "${PILOT_ENV[BUZZ_PRIVATE_KEY]}" ]] || {
    pilot_die 'pilot credentials are empty'; return 1;
  }

  prompt="${PILOT_ENV[BUZZ_ACP_SYSTEM_PROMPT_FILE]}"
  if [[ "$prompt" != /* ]]; then
    prompt="$PILOT_REPO_ROOT/$prompt"
  fi
  [[ -f "$prompt" && -r "$prompt" ]] || { pilot_die 'system prompt is missing'; return 1; }
  PILOT_ENV[BUZZ_ACP_SYSTEM_PROMPT_FILE]="$prompt"
}

pilot_check_secret_permissions() {
  local mode owner
  owner="$(stat -c '%u' "$PILOT_SECRETS_FILE" 2>/dev/null)" || return 0
  mode="$(stat -c '%a' "$PILOT_SECRETS_FILE" 2>/dev/null)" || return 0
  [[ "$owner" == "$UID" ]] || { pilot_die 'secret file must be owned by the current user'; return 1; }
  (( (8#$mode & 077) == 0 )) || { pilot_die 'secret file must not be group/world readable'; return 1; }
}

pilot_prepare_state_dir() {
  umask 077
  mkdir -p "$PILOT_STATE_DIR" || { pilot_die 'unable to create pilot state directory'; return 1; }
  chmod 700 "$PILOT_STATE_DIR" || { pilot_die 'unable to secure pilot state directory'; return 1; }
}

pilot_require_release_binaries() {
  PILOT_BIN_DIR="$PILOT_REPO_ROOT/target/release"
  local binary
  for binary in buzz-relay buzz-acp buzz-agent; do
    [[ -x "$PILOT_BIN_DIR/$binary" ]] || { pilot_die 'required release binary is missing'; return 1; }
  done
}

pilot_load_and_validate() {
  declare -gA PILOT_ENV=()
  pilot_read_file "$PILOT_CONFIG_FILE" configuration || return 1
  pilot_check_secret_permissions || return 1
  pilot_read_file "$PILOT_SECRETS_FILE" secret || return 1
  pilot_validate_config || return 1
  pilot_prepare_state_dir || return 1
  pilot_require_release_binaries || return 1
}

pilot_marker_matches() {
  local marker="$1" expected="$2" pid binary cmdline
  [[ -f "$marker" ]] || return 1
  IFS='|' read -r pid binary < "$marker" || return 1
  [[ "$binary" == "$expected" && "$pid" =~ ^[0-9]+$ ]] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  cmdline="$(tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null || true)"
  [[ "$cmdline" == *"$expected"* ]]
}

pilot_stop_marker() {
  local marker="$1" expected="$2" pid binary
  if [[ -f "$marker" ]]; then
    IFS='|' read -r pid binary < "$marker" || true
    if pilot_marker_matches "$marker" "$expected"; then
      kill "$pid" 2>/dev/null || true
    fi
    rm -f "$marker"
  fi
}
