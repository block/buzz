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

pilot_default_transfer_base_commit() {
  printf '%s' 'b7bb15122e8a2053b545dc2210afc167f6c7a626'
}

pilot_reviewed_prompt_sha256() {
  printf '%s' '2da83d41001a2084463e1c6a147905ddd40c37ec08788819aae4e302090b41ad'
}

pilot_transfer_identity_keys() {
  printf '%s\n' \
    CORE_RELAY_PUBLIC_KEY CORE_RELAY_PRIVATE_KEY \
    CORE_BANKER_PUBLIC_KEY CORE_BANKER_PRIVATE_KEY \
    CORE_AGENT_PUBLIC_KEY CORE_AGENT_PRIVATE_KEY \
    CORE_NON_OWNER_PUBLIC_KEY CORE_NON_OWNER_PRIVATE_KEY
}

pilot_transfer_key_allowed() {
  case "$1" in
    CORE_PILOT_TRANSFER_SCHEMA|CORE_PILOT_SOURCE_COMMIT|CORE_PILOT_BUNDLE_BASE|CORE_PILOT_PROMPT_SHA256|\
    CORE_RELAY_PUBLIC_KEY|CORE_RELAY_PRIVATE_KEY|CORE_BANKER_PUBLIC_KEY|CORE_BANKER_PRIVATE_KEY|\
    CORE_AGENT_PUBLIC_KEY|CORE_AGENT_PRIVATE_KEY|CORE_NON_OWNER_PUBLIC_KEY|CORE_NON_OWNER_PRIVATE_KEY|\
    CORE_RESEARCH_CHANNEL_ID|CORE_SECOND_CHANNEL_ID)
      return 0
      ;;
  esac
  return 1
}

pilot_validate_transfer_identity_values() {
  local key value
  local LC_ALL=C
  local secret_order='fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141'
  local field_prime='fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f'
  for key in $(pilot_transfer_identity_keys); do
    [[ -n "${PILOT_TRANSFER[$key]+set}" && "${PILOT_TRANSFER[$key]}" =~ ^[0-9a-fA-F]{64}$ ]] || {
      pilot_die 'portable identity state is malformed'
      return 1
    }
    PILOT_TRANSFER["$key"]="${PILOT_TRANSFER[$key],,}"
    value="${PILOT_TRANSFER[$key]}"
    case "$key" in
      *_PRIVATE_KEY)
        [[ "$value" != 0000000000000000000000000000000000000000000000000000000000000000 \
           && "$value" < "$secret_order" ]] || {
          pilot_die 'portable private identity scalar is invalid'
          return 1
        }
        ;;
      *_PUBLIC_KEY)
        [[ "$value" < "$field_prime" ]] || {
          pilot_die 'portable public identity coordinate is invalid'
          return 1
        }
        ;;
    esac
  done
  pilot_validate_identity_role_separation PILOT_TRANSFER || return 1
  pilot_validate_identity_keypairs PILOT_TRANSFER
}

pilot_validate_identity_role_separation() {
  local identity_array_name="$1" prefix public_value private_value
  local -n identity_values="$identity_array_name"
  local -A public_values=() private_values=()
  for prefix in CORE_RELAY CORE_BANKER CORE_AGENT CORE_NON_OWNER; do
    public_value="${identity_values[${prefix}_PUBLIC_KEY]:-}"
    private_value="${identity_values[${prefix}_PRIVATE_KEY]:-}"
    public_value="${public_value,,}"
    private_value="${private_value,,}"
    [[ -n "$public_value" && -n "$private_value" ]] || {
      pilot_die 'stable pilot identity roles are incomplete'
      return 1
    }
    [[ -z "${public_values[$public_value]+set}" ]] || {
      pilot_die 'stable pilot public identity roles must be distinct'
      return 1
    }
    [[ -z "${private_values[$private_value]+set}" ]] || {
      pilot_die 'stable pilot private identity roles must be distinct'
      return 1
    }
    public_values["$public_value"]=1
    private_values["$private_value"]=1
  done
}

pilot_validate_identity_keypairs() {
  local identity_array_name="$1" prefix public_value private_value public_der derived_public
  local -n identity_values="$identity_array_name"
  local LC_ALL=C
  local secret_order='fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141'
  local field_prime='fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f'
  command -v openssl >/dev/null 2>&1 && command -v xxd >/dev/null 2>&1 || {
    pilot_die 'OpenSSL and xxd are required to validate stable pilot identity pairs'
    return 1
  }
  for prefix in CORE_RELAY CORE_BANKER CORE_AGENT CORE_NON_OWNER; do
    public_value="${identity_values[${prefix}_PUBLIC_KEY]:-}"
    private_value="${identity_values[${prefix}_PRIVATE_KEY]:-}"
    public_value="${public_value,,}"
    private_value="${private_value,,}"
    [[ "$public_value" =~ ^[0-9a-f]{64}$ && "$public_value" < "$field_prime" \
       && "$private_value" =~ ^[0-9a-f]{64}$ \
       && "$private_value" != 0000000000000000000000000000000000000000000000000000000000000000 \
       && "$private_value" < "$secret_order" ]] || {
      pilot_die 'stable pilot identity key material is invalid'
      return 1
    }
    public_der="$(
      printf '302e0201010420%sa00706052b8104000a' "$private_value" \
        | xxd -r -p \
        | openssl ec -inform DER -pubout -outform DER -conv_form uncompressed 2>/dev/null \
        | xxd -p -c 1000
    )" || {
      pilot_die 'unable to derive a stable pilot public identity'
      return 1
    }
    [[ "$public_der" =~ 04([0-9a-f]{64})[0-9a-f]{64}$ ]] || {
      pilot_die 'derived stable pilot public identity is malformed'
      return 1
    }
    derived_public="${BASH_REMATCH[1]}"
    public_der=
    [[ "$derived_public" == "$public_value" ]] || {
      pilot_die 'stable pilot public/private identity pair does not match'
      return 1
    }
  done
}

pilot_validate_uuid_v4() {
  [[ "$1" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-4[0-9a-fA-F]{3}-[89aAbB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}$ ]]
}

pilot_file_contains_only_text_records() {
  LC_ALL=C tr -d '\11\12\15\40-\176' < "$1" | cmp -s - /dev/null
}

# Validate a private input without following a symlink or silently accepting a
# non-canonical path. The file may be stricter than 0600 (for example 0400),
# but it must never be accessible to the group or world.
pilot_check_private_input_file() {
  local file="$1" label="$2" canonical kind owner mode
  [[ "$file" == /* ]] || { pilot_die "$label path must be absolute"; return 1; }
  [[ -f "$file" && ! -L "$file" ]] || { pilot_die "$label must be a regular non-symlink file"; return 1; }
  canonical="$(realpath -e -- "$file" 2>/dev/null)" || { pilot_die "unable to resolve $label"; return 1; }
  [[ "$canonical" == "$file" ]] || { pilot_die "$label path must be canonical"; return 1; }
  kind="$(stat -c '%F' -- "$canonical" 2>/dev/null)" || { pilot_die "unable to inspect $label"; return 1; }
  owner="$(stat -c '%u' -- "$canonical" 2>/dev/null)" || { pilot_die "unable to inspect $label"; return 1; }
  mode="$(stat -c '%a' -- "$canonical" 2>/dev/null)" || { pilot_die "unable to inspect $label"; return 1; }
  [[ "$kind" == 'regular file' && "$owner" == "$UID" ]] || {
    pilot_die "$label must be a current-user-owned regular file"
    return 1
  }
  (( (8#$mode & 077) == 0 )) || { pilot_die "$label has unsafe permissions"; return 1; }
}

pilot_check_path_outside_repo() {
  local path="$1" repo="$2" label="$3" canonical repo_canonical
  canonical="$(realpath -e -- "$path" 2>/dev/null)" || { pilot_die "unable to resolve $label"; return 1; }
  repo_canonical="$(realpath -e -- "$repo" 2>/dev/null)" || { pilot_die 'unable to resolve repository root'; return 1; }
  case "$canonical" in
    "$repo_canonical"|"$repo_canonical"/*)
      pilot_die "$label must live outside the repository"
      return 1
      ;;
  esac
}

pilot_check_new_external_directory_path() {
  local destination="$1" repo="$2" parent basename parent_canonical expected repo_canonical owner mode
  [[ "$destination" == /* ]] || { pilot_die 'transfer directory path must be absolute'; return 1; }
  [[ ! -e "$destination" && ! -L "$destination" ]] || {
    pilot_die 'transfer directory already exists; refusing to overwrite it'
    return 1
  }
  parent="$(dirname -- "$destination")"
  basename="$(basename -- "$destination")"
  [[ "$basename" != '.' && "$basename" != '..' && "$basename" != '' ]] || {
    pilot_die 'transfer directory path is unsafe'
    return 1
  }
  [[ -d "$parent" && ! -L "$parent" ]] || { pilot_die 'transfer parent must be a real directory'; return 1; }
  parent_canonical="$(realpath -e -- "$parent" 2>/dev/null)" || { pilot_die 'unable to resolve transfer parent'; return 1; }
  expected="$parent_canonical/$basename"
  [[ "$destination" == "$expected" ]] || { pilot_die 'transfer directory path must be canonical'; return 1; }
  repo_canonical="$(realpath -e -- "$repo" 2>/dev/null)" || { pilot_die 'unable to resolve repository root'; return 1; }
  case "$destination" in
    "$repo_canonical"|"$repo_canonical"/*)
      pilot_die 'transfer directory must live outside the repository'
      return 1
      ;;
  esac
  owner="$(stat -c '%u' -- "$parent_canonical" 2>/dev/null)" || { pilot_die 'unable to inspect transfer parent'; return 1; }
  mode="$(stat -c '%a' -- "$parent_canonical" 2>/dev/null)" || { pilot_die 'unable to inspect transfer parent'; return 1; }
  if [[ "$owner" != "$UID" ]] && (( (8#$mode & 01000) == 0 )); then
    pilot_die 'transfer parent is not safely owned'
    return 1
  fi
  if (( (8#$mode & 0022) != 0 && (8#$mode & 01000) == 0 )); then
    pilot_die 'transfer parent has unsafe permissions'
    return 1
  fi
}

pilot_bundle_prerequisite() {
  local bundle="$1" line prerequisite='' count=0 bundle_fd
  exec {bundle_fd}<"$bundle" || return 1
  IFS= read -r line <&"$bundle_fd" || { exec {bundle_fd}<&-; return 1; }
  [[ "$line" == '# v2 git bundle' || "$line" == '# v3 git bundle' ]] || {
    exec {bundle_fd}<&-
    return 1
  }
  while IFS= read -r line <&"$bundle_fd"; do
    [[ -n "$line" ]] || break
    if [[ "$line" =~ ^-([0-9a-f]{40})[[:space:]] ]]; then
      prerequisite="${BASH_REMATCH[1]}"
      count=$((count + 1))
    fi
  done
  exec {bundle_fd}<&-
  [[ $count -eq 1 ]] || return 1
  printf '%s' "$prerequisite"
}

pilot_bundle_head() {
  local bundle="$1" line head='' count=0
  while IFS= read -r line; do
    [[ "$line" =~ ^([0-9a-f]{40})[[:space:]]HEAD$ ]] || return 1
    head="${BASH_REMATCH[1]}"
    count=$((count + 1))
  done < <(git bundle list-heads "$bundle" 2>/dev/null)
  [[ $count -eq 1 ]] || return 1
  printf '%s' "$head"
}

pilot_verify_transfer_manifest() {
  local directory="$1" manifest="$2" line digest filename actual
  declare -A manifest_entries=()
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" =~ ^([0-9a-f]{64})[[:space:]][[:space:]](core-pilot\.bundle|core-pilot-state\.gpg)$ ]] || {
      pilot_die 'transfer checksum manifest is malformed'
      return 1
    }
    digest="${BASH_REMATCH[1]}"
    filename="${BASH_REMATCH[2]}"
    [[ -z "${manifest_entries[$filename]+set}" ]] || {
      pilot_die 'transfer checksum manifest contains a duplicate entry'
      return 1
    }
    manifest_entries["$filename"]="$digest"
  done < "$manifest"
  [[ ${#manifest_entries[@]} -eq 2 \
     && -n "${manifest_entries[core-pilot.bundle]+set}" \
     && -n "${manifest_entries[core-pilot-state.gpg]+set}" ]] || {
    pilot_die 'transfer checksum manifest is incomplete'
    return 1
  }
  for filename in core-pilot.bundle core-pilot-state.gpg; do
    actual="$(sha256sum -- "$directory/$filename" 2>/dev/null)" || {
      pilot_die 'unable to verify transfer artifact checksum'
      return 1
    }
    [[ "${actual%% *}" == "${manifest_entries[$filename]}" ]] || {
      pilot_die 'transfer artifact checksum mismatch'
      return 1
    }
  done
}

pilot_read_transfer_file() {
  local file="$1" line key value required
  declare -gA PILOT_TRANSFER=()
  [[ -f "$file" && ! -L "$file" && -r "$file" ]] || {
    pilot_die 'decrypted portable state is unavailable'
    return 1
  }
  pilot_file_contains_only_text_records "$file" || {
    pilot_die 'portable state contains a binary record'
    return 1
  }
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    [[ "$line" =~ ^([A-Z][A-Z0-9_]*)=([^[:space:]]*)$ ]] || {
      pilot_die 'portable state contains a malformed record'
      return 1
    }
    key="${BASH_REMATCH[1]}"
    value="${BASH_REMATCH[2]}"
    pilot_transfer_key_allowed "$key" || {
      pilot_die 'portable state contains an unsupported field'
      return 1
    }
    [[ -z "${PILOT_TRANSFER[$key]+set}" ]] || {
      pilot_die 'portable state contains a duplicate field'
      return 1
    }
    [[ -n "$value" ]] || { pilot_die 'portable state contains an empty field'; return 1; }
    PILOT_TRANSFER["$key"]="$value"
  done < "$file"

  required=(
    CORE_PILOT_TRANSFER_SCHEMA CORE_PILOT_SOURCE_COMMIT CORE_PILOT_BUNDLE_BASE CORE_PILOT_PROMPT_SHA256
    CORE_RELAY_PUBLIC_KEY CORE_RELAY_PRIVATE_KEY CORE_BANKER_PUBLIC_KEY CORE_BANKER_PRIVATE_KEY
    CORE_AGENT_PUBLIC_KEY CORE_AGENT_PRIVATE_KEY CORE_NON_OWNER_PUBLIC_KEY CORE_NON_OWNER_PRIVATE_KEY
    CORE_RESEARCH_CHANNEL_ID CORE_SECOND_CHANNEL_ID
  )
  [[ ${#PILOT_TRANSFER[@]} -eq ${#required[@]} ]] || {
    pilot_die 'portable state is incomplete'
    return 1
  }
  for key in "${required[@]}"; do
    [[ -n "${PILOT_TRANSFER[$key]+set}" ]] || { pilot_die 'portable state is incomplete'; return 1; }
  done
  [[ "${PILOT_TRANSFER[CORE_PILOT_TRANSFER_SCHEMA]}" == 1 ]] || {
    pilot_die 'portable state schema is unsupported'
    return 1
  }
  [[ "${PILOT_TRANSFER[CORE_PILOT_SOURCE_COMMIT]}" =~ ^[0-9a-f]{40}$ \
     && "${PILOT_TRANSFER[CORE_PILOT_BUNDLE_BASE]}" =~ ^[0-9a-f]{40}$ \
     && "${PILOT_TRANSFER[CORE_PILOT_PROMPT_SHA256]}" =~ ^[0-9a-f]{64}$ ]] || {
    pilot_die 'portable state metadata is malformed'
    return 1
  }
  pilot_validate_transfer_identity_values || return 1
  pilot_validate_uuid_v4 "${PILOT_TRANSFER[CORE_RESEARCH_CHANNEL_ID]}" || {
    pilot_die 'portable channel state contains an invalid UUID'
    return 1
  }
  pilot_validate_uuid_v4 "${PILOT_TRANSFER[CORE_SECOND_CHANNEL_ID]}" || {
    pilot_die 'portable channel state contains an invalid UUID'
    return 1
  }
  [[ "${PILOT_TRANSFER[CORE_RESEARCH_CHANNEL_ID],,}" \
     != "${PILOT_TRANSFER[CORE_SECOND_CHANNEL_ID],,}" ]] || {
    pilot_die 'portable channel UUIDs must be distinct'
    return 1
  }
  PILOT_TRANSFER[CORE_RESEARCH_CHANNEL_ID]="${PILOT_TRANSFER[CORE_RESEARCH_CHANNEL_ID],,}"
  PILOT_TRANSFER[CORE_SECOND_CHANNEL_ID]="${PILOT_TRANSFER[CORE_SECOND_CHANNEL_ID],,}"
}

pilot_check_private_directory() {
  local directory="$1" label="$2" canonical kind owner mode
  [[ "$directory" == /* ]] || { pilot_die "$label path must be absolute"; return 1; }
  [[ -d "$directory" && ! -L "$directory" ]] || { pilot_die "$label must be a real directory"; return 1; }
  canonical="$(realpath -e -- "$directory" 2>/dev/null)" || { pilot_die "unable to resolve $label"; return 1; }
  [[ "$canonical" == "$directory" ]] || { pilot_die "$label path must be canonical"; return 1; }
  kind="$(stat -c '%F' -- "$canonical" 2>/dev/null)" || { pilot_die "unable to inspect $label"; return 1; }
  owner="$(stat -c '%u' -- "$canonical" 2>/dev/null)" || { pilot_die "unable to inspect $label"; return 1; }
  mode="$(stat -c '%a' -- "$canonical" 2>/dev/null)" || { pilot_die "unable to inspect $label"; return 1; }
  [[ "$kind" == 'directory' && "$owner" == "$UID" ]] || {
    pilot_die "$label must be current-user owned"
    return 1
  }
  (( (8#$mode & 077) == 0 )) || { pilot_die "$label has unsafe permissions"; return 1; }
}

pilot_check_temporary_parent() {
  local directory="$1" canonical owner mode
  [[ "$directory" == /* && -d "$directory" && ! -L "$directory" ]] || {
    pilot_die 'temporary workspace parent is unsafe'
    return 1
  }
  canonical="$(realpath -e -- "$directory" 2>/dev/null)" || {
    pilot_die 'unable to resolve temporary workspace parent'
    return 1
  }
  [[ "$canonical" == "$directory" ]] || {
    pilot_die 'temporary workspace parent path must be canonical'
    return 1
  }
  owner="$(stat -c '%u' -- "$canonical" 2>/dev/null)" || {
    pilot_die 'unable to inspect temporary workspace parent'
    return 1
  }
  mode="$(stat -c '%a' -- "$canonical" 2>/dev/null)" || {
    pilot_die 'unable to inspect temporary workspace parent'
    return 1
  }
  if [[ "$owner" != "$UID" ]] && (( (8#$mode & 01000) == 0 )); then
    pilot_die 'temporary workspace parent is not safely owned'
    return 1
  fi
  if (( (8#$mode & 0022) != 0 && (8#$mode & 01000) == 0 )); then
    pilot_die 'temporary workspace parent has unsafe permissions'
    return 1
  fi
}

pilot_prepare_private_destination_directory() {
  local directory="$1" label="$2" repo="$3" normalized repo_canonical cursor owner mode
  [[ "$directory" == /* ]] || { pilot_die "$label path must be absolute"; return 1; }
  normalized="$(realpath -m -- "$directory" 2>/dev/null)" || { pilot_die "unable to normalize $label"; return 1; }
  [[ "$normalized" == "$directory" ]] || { pilot_die "$label path must be canonical"; return 1; }
  repo_canonical="$(realpath -e -- "$repo" 2>/dev/null)" || { pilot_die 'unable to resolve repository root'; return 1; }
  case "$directory" in
    "$repo_canonical"|"$repo_canonical"/*)
      pilot_die "$label must live outside the repository"
      return 1
      ;;
  esac
  if [[ -e "$directory" || -L "$directory" ]]; then
    pilot_check_private_directory "$directory" "$label"
    return
  fi
  cursor="$directory"
  while [[ ! -e "$cursor" && ! -L "$cursor" ]]; do
    [[ "$cursor" != / ]] || break
    cursor="$(dirname -- "$cursor")"
  done
  [[ -d "$cursor" && ! -L "$cursor" ]] || { pilot_die "$label has an unsafe ancestor"; return 1; }
  [[ "$(realpath -e -- "$cursor" 2>/dev/null)" == "$cursor" ]] || {
    pilot_die "$label has a non-canonical ancestor"
    return 1
  }
  owner="$(stat -c '%u' -- "$cursor" 2>/dev/null)" || { pilot_die "unable to inspect $label ancestor"; return 1; }
  mode="$(stat -c '%a' -- "$cursor" 2>/dev/null)" || { pilot_die "unable to inspect $label ancestor"; return 1; }
  [[ "$owner" == "$UID" && $((8#$mode & 0022)) -eq 0 ]] || {
    pilot_die "$label has an unsafe ancestor"
    return 1
  }
  umask 077
  mkdir -p -- "$directory" || { pilot_die "unable to create $label"; return 1; }
  chmod 700 -- "$directory" || { pilot_die "unable to secure $label"; return 1; }
  pilot_check_private_directory "$directory" "$label"
}

pilot_check_existing_destination_file() {
  local file="$1" label="$2" canonical kind owner mode
  [[ ! -L "$file" ]] || { pilot_die "$label must not be a symlink"; return 1; }
  [[ -e "$file" ]] || return 0
  [[ -f "$file" ]] || { pilot_die "$label must be a regular file"; return 1; }
  canonical="$(realpath -e -- "$file" 2>/dev/null)" || { pilot_die "unable to resolve $label"; return 1; }
  [[ "$canonical" == "$file" ]] || { pilot_die "$label path must be canonical"; return 1; }
  kind="$(stat -c '%F' -- "$canonical" 2>/dev/null)" || { pilot_die "unable to inspect $label"; return 1; }
  owner="$(stat -c '%u' -- "$canonical" 2>/dev/null)" || { pilot_die "unable to inspect $label"; return 1; }
  mode="$(stat -c '%a' -- "$canonical" 2>/dev/null)" || { pilot_die "unable to inspect $label"; return 1; }
  [[ "$kind" == 'regular file' && "$owner" == "$UID" ]] || {
    pilot_die "$label must be current-user owned"
    return 1
  }
  (( (8#$mode & 077) == 0 )) || { pilot_die "$label has unsafe permissions"; return 1; }
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
  pilot_file_contains_only_text_records "$file" || pilot_die "$kind file contains a binary record" || return 1

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
  pilot_require_value DATABASE_URL 'postgres://buzz:buzz_dev@127.0.0.1:15432/buzz' || return 1
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
  [[ "${prompt_hash%% *}" == "$(pilot_reviewed_prompt_sha256)" ]] || {
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
  pilot_check_private_input_file "$PILOT_CHANNELS_FILE" 'generated channel state' || return 1
  pilot_file_contains_only_text_records "$PILOT_CHANNELS_FILE" || {
    pilot_die 'generated channel state contains a binary record'; return 1;
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
    PILOT_CHANNELS["$key"]="${value,,}"
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
  pilot_validate_identity_role_separation PILOT_ENV || return 1
  pilot_validate_identity_keypairs PILOT_ENV || return 1
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
