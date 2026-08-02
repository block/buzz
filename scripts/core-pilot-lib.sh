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
  PILOT_CHANNELS_FILE="$PILOT_STATE_DIR/channels.env"
}

pilot_config_key_allowed() {
  case "$1" in
    BUZZ_RELAY_URL|BUZZ_BIND_ADDR|DATABASE_URL|REDIS_URL|BUZZ_REQUIRE_AUTH_TOKEN|BUZZ_REQUIRE_RELAY_MEMBERSHIP|BUZZ_GIT_ENABLED|\
    BUZZ_AGENT_PROVIDER|BUZZ_AGENT_MODEL|OPENAI_COMPAT_API|OPENAI_COMPAT_BASE_URL|OPENAI_COMPAT_MODEL|\
    BUZZ_AGENT_THINKING_EFFORT|BUZZ_AGENT_WEB_SEARCH|BUZZ_AGENT_NO_HINTS|BUZZ_AGENT_REQUIRE_REPLY|\
    BUZZ_ACP_SYSTEM_PROMPT_FILE|BUZZ_ACP_NO_BASE_PROMPT|BUZZ_ACP_NO_MEMORY|BUZZ_ACP_AGENT_COMMAND|\
    BUZZ_ACP_AGENT_ARGS|BUZZ_ACP_MODEL|BUZZ_ACP_MCP_COMMAND|BUZZ_ACP_PUBLISH_AGENT_OUTPUT|BUZZ_ACP_AGENTS|\
    BUZZ_ACP_HEARTBEAT_INTERVAL|BUZZ_ACP_SUBSCRIBE|BUZZ_ACP_KINDS|BUZZ_ACP_CHANNELS|\
    BUZZ_ACP_RESPOND_TO|BUZZ_ACP_AGENT_OWNER|BUZZ_ACP_DEDUP|BUZZ_ACP_MULTIPLE_EVENT_HANDLING)
      return 0
      ;;
  esac
  return 1
}

pilot_secret_key_allowed() {
  case "$1" in
    OPENAI_COMPAT_API_KEY|CORE_RELAY_PUBLIC_KEY|CORE_RELAY_PRIVATE_KEY|\
    CORE_BANKER_PUBLIC_KEY|CORE_BANKER_PRIVATE_KEY|CORE_AGENT_PUBLIC_KEY|\
    CORE_AGENT_PRIVATE_KEY|CORE_NON_OWNER_PUBLIC_KEY|CORE_NON_OWNER_PRIVATE_KEY)
      return 0
      ;;
  esac
  return 1
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
    if [[ "$key" != 'BUZZ_ACP_MCP_COMMAND' && "$key" != 'OPENAI_COMPAT_API_KEY' && -z "$value" ]]; then
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
  local required key prompt prompt_canonical reviewed_prompt prompt_hash channels owner normalized_url
  required=(
    BUZZ_RELAY_URL BUZZ_BIND_ADDR DATABASE_URL REDIS_URL BUZZ_REQUIRE_AUTH_TOKEN BUZZ_REQUIRE_RELAY_MEMBERSHIP BUZZ_GIT_ENABLED
    BUZZ_AGENT_PROVIDER BUZZ_AGENT_MODEL OPENAI_COMPAT_API OPENAI_COMPAT_BASE_URL OPENAI_COMPAT_MODEL
    BUZZ_AGENT_THINKING_EFFORT BUZZ_AGENT_WEB_SEARCH BUZZ_AGENT_NO_HINTS BUZZ_AGENT_REQUIRE_REPLY
    BUZZ_ACP_SYSTEM_PROMPT_FILE BUZZ_ACP_NO_BASE_PROMPT BUZZ_ACP_NO_MEMORY BUZZ_ACP_AGENT_COMMAND
    BUZZ_ACP_AGENT_ARGS BUZZ_ACP_MODEL BUZZ_ACP_MCP_COMMAND BUZZ_ACP_PUBLISH_AGENT_OUTPUT BUZZ_ACP_AGENTS
    BUZZ_ACP_HEARTBEAT_INTERVAL BUZZ_ACP_SUBSCRIBE BUZZ_ACP_KINDS BUZZ_ACP_CHANNELS
    BUZZ_ACP_RESPOND_TO BUZZ_ACP_AGENT_OWNER BUZZ_ACP_DEDUP BUZZ_ACP_MULTIPLE_EVENT_HANDLING
    OPENAI_COMPAT_API_KEY CORE_RELAY_PUBLIC_KEY CORE_RELAY_PRIVATE_KEY
    CORE_BANKER_PUBLIC_KEY CORE_BANKER_PRIVATE_KEY CORE_AGENT_PUBLIC_KEY CORE_AGENT_PRIVATE_KEY
    CORE_NON_OWNER_PUBLIC_KEY CORE_NON_OWNER_PRIVATE_KEY
  )
  for key in "${required[@]}"; do
    pilot_require "$key" || return 1
  done

  pilot_require_value BUZZ_RELAY_URL 'ws://127.0.0.1:3000' || return 1
  pilot_require_value BUZZ_BIND_ADDR '127.0.0.1:3000' || return 1
  pilot_require_value DATABASE_URL 'postgres://buzz:buzz_dev@127.0.0.1:5432/buzz' || return 1
  pilot_require_value REDIS_URL 'redis://127.0.0.1:6379' || return 1
  pilot_require_value BUZZ_REQUIRE_AUTH_TOKEN false || return 1
  pilot_require_value BUZZ_REQUIRE_RELAY_MEMBERSHIP true || return 1
  pilot_require_value BUZZ_GIT_ENABLED false || return 1
  pilot_require_value BUZZ_AGENT_PROVIDER openai || return 1
  pilot_require_value BUZZ_AGENT_MODEL gpt-5.6-terra || return 1
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
  pilot_require_value BUZZ_ACP_MODEL gpt-5.6-terra || return 1
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
  [[ "$channels" != '11111111-1111-4111-8111-111111111111' ]] || {
    pilot_die 'replace the template channel before launch'; return 1;
  }
  owner="${PILOT_ENV[BUZZ_ACP_AGENT_OWNER]}"
  [[ "$owner" =~ ^[0-9a-fA-F]{64}$ ]] || { pilot_die 'pilot owner must be a public key'; return 1; }
  [[ "${owner,,}" != '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' ]] || {
    pilot_die 'replace the template owner before launch'; return 1;
  }
  [[ -n "${PILOT_ENV[OPENAI_COMPAT_API_KEY]}" ]] || {
    pilot_die 'OpenAI credential is unavailable; bootstrap is allowed but ACP start is gated'; return 1;
  }
  [[ "${owner,,}" == "${PILOT_ENV[CORE_BANKER_PUBLIC_KEY],,}" ]] || {
    pilot_die 'configured owner does not match the stable banker identity'; return 1;
  }
  [[ "$channels" == "${PILOT_CHANNELS[CORE_RESEARCH_CHANNEL_ID]:-}" ]] || {
    pilot_die 'configured channel does not match generated pilot state'; return 1;
  }
  [[ -n "${PILOT_CHANNELS[CORE_SECOND_CHANNEL_ID]:-}" && "$channels" != "${PILOT_CHANNELS[CORE_SECOND_CHANNEL_ID]}" ]] || {
    pilot_die 'second-channel control state is missing or unsafe'; return 1;
  }

  prompt="${PILOT_ENV[BUZZ_ACP_SYSTEM_PROMPT_FILE]}"
  if [[ "$prompt" != /* ]]; then
    prompt="$PILOT_REPO_ROOT/$prompt"
  fi
  reviewed_prompt="$PILOT_REPO_ROOT/config/core-pilot/core-research-partner.md"
  prompt_canonical="$(realpath -e -- "$prompt" 2>/dev/null)" || { pilot_die 'system prompt is missing'; return 1; }
  reviewed_prompt="$(realpath -e -- "$reviewed_prompt" 2>/dev/null)" || { pilot_die 'reviewed system prompt is missing'; return 1; }
  [[ "$prompt_canonical" == "$reviewed_prompt" && -f "$prompt_canonical" && -r "$prompt_canonical" ]] || {
    pilot_die 'system prompt is not the reviewed Core prompt'; return 1;
  }
  prompt_hash="$(sha256sum -- "$prompt_canonical" 2>/dev/null)" || { pilot_die 'unable to verify system prompt'; return 1; }
  [[ "${prompt_hash%% *}" == '2da83d41001a2084463e1c6a147905ddd40c37ec08788819aae4e302090b41ad' ]] || {
    pilot_die 'reviewed system prompt failed integrity verification'; return 1;
  }
  PILOT_ENV[BUZZ_ACP_SYSTEM_PROMPT_FILE]="$prompt_canonical"
}

pilot_check_secret_permissions() {
  local mode owner kind canonical repo_canonical
  [[ -f "$PILOT_SECRETS_FILE" && ! -L "$PILOT_SECRETS_FILE" ]] || {
    pilot_die 'secret file must be a regular non-symlink file'; return 1;
  }
  canonical="$(realpath -e -- "$PILOT_SECRETS_FILE" 2>/dev/null)" || {
    pilot_die 'unable to resolve secret file'; return 1;
  }
  [[ "$canonical" == "$PILOT_SECRETS_FILE" ]] || {
    pilot_die 'secret file path must be canonical'; return 1;
  }
  repo_canonical="$(realpath -e -- "$PILOT_REPO_ROOT" 2>/dev/null)" || {
    pilot_die 'unable to resolve repository root'; return 1;
  }
  case "$canonical" in
    "$repo_canonical"|"$repo_canonical"/*)
      pilot_die 'secret file must live outside the repository'
      return 1
      ;;
  esac
  kind="$(stat -c '%F' -- "$canonical" 2>/dev/null)" || { pilot_die 'unable to inspect secret file'; return 1; }
  owner="$(stat -c '%u' -- "$canonical" 2>/dev/null)" || { pilot_die 'unable to inspect secret file'; return 1; }
  mode="$(stat -c '%a' -- "$canonical" 2>/dev/null)" || { pilot_die 'unable to inspect secret file'; return 1; }
  [[ "$kind" == 'regular file' ]] || { pilot_die 'secret file must be regular'; return 1; }
  [[ "$owner" == "$UID" ]] || { pilot_die 'secret file must be owned by the current user'; return 1; }
  (( (8#$mode & 077) == 0 )) || { pilot_die 'secret file must not be group/world readable'; return 1; }
}

pilot_prepare_state_dir() {
  umask 077
  mkdir -p "$PILOT_STATE_DIR" || { pilot_die 'unable to create pilot state directory'; return 1; }
  chmod 700 "$PILOT_STATE_DIR" || { pilot_die 'unable to secure pilot state directory'; return 1; }
}

# Clear inherited exported variables using Bash builtins only. Call this inside
# a subshell immediately before exporting the exact target environment and
# directly execing the real binary. Secrets therefore never appear in argv.
pilot_clear_environment() {
  local variable
  while IFS= read -r variable; do
    unset "$variable" 2>/dev/null || true
  done < <(compgen -e)
}

pilot_require_release_binaries() {
  PILOT_BIN_DIR="$PILOT_REPO_ROOT/target/release"
  local binary
  for binary in buzz-relay buzz-admin buzz-acp buzz-agent buzz; do
    [[ -x "$PILOT_BIN_DIR/$binary" ]] || { pilot_die 'required release binary is missing'; return 1; }
  done
}

pilot_validate_nostr_key() {
  local private_key="$1" status
  set +e
  (
    pilot_clear_environment
    export PATH="$PILOT_BIN_DIR:/usr/bin:/bin"
    export BUZZ_PRIVATE_KEY="$private_key"
    export BUZZ_RELAY_URL=ws://127.0.0.1:1
    exec "$PILOT_BIN_DIR/buzz" --format compact users get
  ) >/dev/null 2>&1
  status=$?
  set -e
  [[ $status -eq 2 ]] || { pilot_die 'agent Nostr private key is invalid'; return 1; }
}

pilot_load_channels() {
  local line key value
  declare -gA PILOT_CHANNELS=()
  [[ -f "$PILOT_CHANNELS_FILE" && ! -L "$PILOT_CHANNELS_FILE" ]] || {
    pilot_die 'generated channel state is missing'; return 1;
  }
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" =~ ^(CORE_RESEARCH_CHANNEL_ID|CORE_SECOND_CHANNEL_ID)=([0-9a-fA-F-]+)$ ]] || {
      pilot_die 'generated channel state is malformed'; return 1;
    }
    key="${BASH_REMATCH[1]}"; value="${BASH_REMATCH[2]}"
    [[ -z "${PILOT_CHANNELS[$key]+set}" ]] || { pilot_die 'generated channel state has duplicate keys'; return 1; }
    [[ "$value" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-4[0-9a-fA-F]{3}-[89aAbB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}$ ]] || {
      pilot_die 'generated channel state contains an invalid UUID'; return 1;
    }
    PILOT_CHANNELS["$key"]="$value"
  done < "$PILOT_CHANNELS_FILE"
  [[ ${#PILOT_CHANNELS[@]} -eq 2 ]] || { pilot_die 'generated channel state is incomplete'; return 1; }
}

pilot_load_and_validate() {
  declare -gA PILOT_ENV=()
  pilot_read_file "$PILOT_CONFIG_FILE" configuration || return 1
  pilot_check_secret_permissions || return 1
  pilot_read_file "$PILOT_SECRETS_FILE" secret || return 1
  pilot_load_channels || return 1
  pilot_validate_config || return 1
  pilot_prepare_state_dir || return 1
  pilot_require_release_binaries || return 1
  pilot_validate_nostr_key "${PILOT_ENV[CORE_RELAY_PRIVATE_KEY]}" || return 1
  pilot_validate_nostr_key "${PILOT_ENV[CORE_BANKER_PRIVATE_KEY]}" || return 1
  pilot_validate_nostr_key "${PILOT_ENV[CORE_AGENT_PRIVATE_KEY]}" || return 1
  pilot_validate_nostr_key "${PILOT_ENV[CORE_NON_OWNER_PRIVATE_KEY]}" || return 1
}

pilot_process_start_time() {
  local pid="$1" stat_line remainder
  stat_line="$(<"/proc/$pid/stat")" 2>/dev/null || return 1
  remainder="${stat_line##*) }"
  awk '{print $20}' <<< "$remainder"
}

pilot_file_identity() {
  stat -Lc '%d:%i' -- "$1" 2>/dev/null
}

pilot_cmdline_has_exact_arg() {
  local pid="$1" expected="$2" arg
  while IFS= read -r -d '' arg; do
    [[ "$arg" == "$expected" ]] && return 0
  done < "/proc/$pid/cmdline" 2>/dev/null
  return 1
}

pilot_write_marker() {
  local marker="$1" pid="$2" expected="$3" expected_path start_time binary_id exe_id proc_exe
  expected_path="$(realpath -e -- "$expected" 2>/dev/null)" || return 1
  start_time="$(pilot_process_start_time "$pid")" || return 1
  binary_id="$(pilot_file_identity "$expected_path")" || return 1
  proc_exe="$(readlink -f -- "/proc/$pid/exe" 2>/dev/null)" || return 1
  exe_id="$(pilot_file_identity "$proc_exe")" || return 1
  [[ "$proc_exe" == "$expected_path" ]] || pilot_cmdline_has_exact_arg "$pid" "$expected_path" || return 1
  printf 'v1|%s|%s|%s|%s|%s\n' "$pid" "$start_time" "$expected_path" "$binary_id" "$exe_id" > "$marker"
}

pilot_marker_matches() {
  local marker="$1" expected="$2" version pid start_time binary binary_id exe_id extra
  local expected_path current_start current_binary_id proc_exe current_exe_id
  [[ -f "$marker" && ! -L "$marker" ]] || return 1
  IFS='|' read -r version pid start_time binary binary_id exe_id extra < "$marker" || return 1
  [[ "$version" == v1 && -z "${extra:-}" && "$pid" =~ ^[0-9]+$ && "$start_time" =~ ^[0-9]+$ ]] || return 1
  expected_path="$(realpath -e -- "$expected" 2>/dev/null)" || return 1
  [[ "$binary" == "$expected_path" ]] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  current_start="$(pilot_process_start_time "$pid")" || return 1
  [[ "$current_start" == "$start_time" ]] || return 1
  current_binary_id="$(pilot_file_identity "$expected_path")" || return 1
  [[ "$current_binary_id" == "$binary_id" ]] || return 1
  proc_exe="$(readlink -f -- "/proc/$pid/exe" 2>/dev/null)" || return 1
  current_exe_id="$(pilot_file_identity "$proc_exe")" || return 1
  [[ "$current_exe_id" == "$exe_id" ]] || return 1
  [[ "$proc_exe" == "$expected_path" ]] || pilot_cmdline_has_exact_arg "$pid" "$expected_path"
}

pilot_stop_marker() {
  local marker="$1" expected="$2" version pid rest
  if [[ -f "$marker" && ! -L "$marker" ]]; then
    IFS='|' read -r version pid rest < "$marker" || true
    if pilot_marker_matches "$marker" "$expected"; then
      pilot_marker_matches "$marker" "$expected" && kill -TERM "$pid" 2>/dev/null || true
      for _ in $(seq 1 50); do
        pilot_marker_matches "$marker" "$expected" || break
        sleep 0.1
      done
      if pilot_marker_matches "$marker" "$expected"; then
        pilot_marker_matches "$marker" "$expected" && kill -KILL "$pid" 2>/dev/null || true
        for _ in $(seq 1 20); do
          pilot_marker_matches "$marker" "$expected" || break
          sleep 0.1
        done
      fi
    fi
    rm -f "$marker"
  fi
}
