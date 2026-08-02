#!/usr/bin/env bash
# Deterministically bootstrap stable local identities, closed relay membership,
# profiles, and two synthetic pilot channels without requiring an OpenAI key.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PILOT_REPO_ROOT="$(cd "$script_dir/.." && pwd)"
# This is a repository-owned helper, not a user configuration file.
source "$script_dir/core-pilot-lib.sh"

pilot_parse_paths "$@"
PILOT_BIN_DIR="$PILOT_REPO_ROOT/target/release"
pilot_require_release_binaries
pilot_prepare_state_dir

secret_parent="$(dirname "$PILOT_SECRETS_FILE")"
umask 077
mkdir -p "$secret_parent"
chmod 700 "$secret_parent"
secret_parent_canonical="$(realpath -e -- "$secret_parent" 2>/dev/null)" || { pilot_die 'unable to resolve secret directory'; exit 1; }
repo_canonical="$(realpath -e -- "$PILOT_REPO_ROOT" 2>/dev/null)" || { pilot_die 'unable to resolve repository root'; exit 1; }
[[ "$secret_parent_canonical/$(basename "$PILOT_SECRETS_FILE")" == "$PILOT_SECRETS_FILE" ]] || {
  pilot_die 'secret path must be canonical'; exit 1;
}
case "$PILOT_SECRETS_FILE" in
  "$repo_canonical"|"$repo_canonical"/*)
    pilot_die 'secret file must live outside the repository'; exit 1 ;;
esac

generate_pair() {
  local prefix="$1" generated public secret
  generated="$("$PILOT_BIN_DIR/buzz-admin" generate-key)" || return 1
  public="$(sed -n 's/^Public key:[[:space:]]*//p' <<< "$generated")"
  secret="$(sed -n 's/^Secret key:[[:space:]]*//p' <<< "$generated")"
  [[ "$public" =~ ^[0-9a-fA-F]{64}$ && -n "$secret" && "$secret" != *[[:space:]]* ]] || {
    generated= public= secret=
    pilot_die 'key generation produced an invalid record'
    return 1
  }
  printf '%s_PUBLIC_KEY=%s\n%s_PRIVATE_KEY=%s\n' "$prefix" "${public,,}" "$prefix" "$secret" >> "$identity_tmp"
  generated= public= secret=
}

if [[ ! -e "$PILOT_SECRETS_FILE" ]]; then
  identity_tmp="$(mktemp "$secret_parent/.agent.env.XXXXXX")"
  trap 'rm -f "${identity_tmp:-}"' EXIT
  printf 'OPENAI_COMPAT_API_KEY=\n' > "$identity_tmp"
  generate_pair CORE_RELAY
  generate_pair CORE_BANKER
  generate_pair CORE_AGENT
  generate_pair CORE_NON_OWNER
  chmod 600 "$identity_tmp"
  mv "$identity_tmp" "$PILOT_SECRETS_FILE"
  identity_tmp=
  trap - EXIT
fi

declare -gA PILOT_ENV=()
pilot_check_secret_permissions
pilot_read_file "$PILOT_SECRETS_FILE" secret
for key in CORE_RELAY_PUBLIC_KEY CORE_RELAY_PRIVATE_KEY CORE_BANKER_PUBLIC_KEY \
  CORE_BANKER_PRIVATE_KEY CORE_AGENT_PUBLIC_KEY CORE_AGENT_PRIVATE_KEY \
  CORE_NON_OWNER_PUBLIC_KEY CORE_NON_OWNER_PRIVATE_KEY OPENAI_COMPAT_API_KEY; do
  pilot_require "$key"
done
for key in CORE_RELAY_PUBLIC_KEY CORE_BANKER_PUBLIC_KEY CORE_AGENT_PUBLIC_KEY CORE_NON_OWNER_PUBLIC_KEY; do
  [[ "${PILOT_ENV[$key]}" =~ ^[0-9a-fA-F]{64}$ ]] || { pilot_die 'stable public identity is malformed'; exit 1; }
done
pilot_validate_nostr_key "${PILOT_ENV[CORE_RELAY_PRIVATE_KEY]}"
pilot_validate_nostr_key "${PILOT_ENV[CORE_BANKER_PRIVATE_KEY]}"
pilot_validate_nostr_key "${PILOT_ENV[CORE_AGENT_PRIVATE_KEY]}"
pilot_validate_nostr_key "${PILOT_ENV[CORE_NON_OWNER_PRIVATE_KEY]}"

command -v docker >/dev/null 2>&1 || { pilot_die 'Docker is required to bootstrap the pilot'; exit 1; }
command -v jq >/dev/null 2>&1 || { pilot_die 'jq is required to bootstrap the pilot'; exit 1; }
cd "$PILOT_REPO_ROOT"
docker compose up -d postgres redis minio minio-init
for container in buzz-postgres buzz-redis buzz-minio; do
  healthy=false
  for _ in $(seq 1 60); do
    if [[ "$(docker inspect --format='{{.State.Health.Status}}' "$container" 2>/dev/null || true)" == healthy ]]; then
      healthy=true
      break
    fi
    sleep 2
  done
  [[ "$healthy" == true ]] || { pilot_die 'local infrastructure did not become healthy'; exit 1; }
done

admin_env=(
  env -i
  "PATH=$PILOT_BIN_DIR:/usr/bin:/bin"
  DATABASE_URL=postgres://buzz:buzz_dev@127.0.0.1:5432/buzz
  REDIS_URL=redis://127.0.0.1:6379
  RELAY_URL=ws://127.0.0.1:3000
  "BUZZ_RELAY_PRIVATE_KEY=${PILOT_ENV[CORE_RELAY_PRIVATE_KEY]}"
)
"${admin_env[@]}" "$PILOT_BIN_DIR/buzz-admin" migrate >> "$PILOT_STATE_DIR/bootstrap.log" 2>&1

relay_marker="$PILOT_STATE_DIR/relay.pid"
relay_bin="$PILOT_BIN_DIR/buzz-relay"
relay_ready() {
  pilot_marker_matches "$relay_marker" "$relay_bin" \
    && [[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:3000/_readiness || true)" == 200 ]]
}
if ! relay_ready; then
  pilot_stop_marker "$relay_marker" "$relay_bin"
  if command -v ss >/dev/null 2>&1; then
    listeners="$(ss -H -ltn 'sport = :3000' 2>/dev/null)" || { pilot_die 'unable to inspect relay port'; exit 1; }
    [[ -z "$listeners" ]] || { pilot_die 'relay port is occupied by a non-pilot process'; exit 1; }
  fi
  nohup env -i \
    "PATH=$PILOT_BIN_DIR:/usr/bin:/bin" \
    RUST_LOG=buzz_relay=info \
    DATABASE_URL=postgres://buzz:buzz_dev@127.0.0.1:5432/buzz \
    REDIS_URL=redis://127.0.0.1:6379 \
    RELAY_URL=ws://127.0.0.1:3000 \
    BUZZ_BIND_ADDR=127.0.0.1:3000 \
    BUZZ_REQUIRE_AUTH_TOKEN=false \
    BUZZ_REQUIRE_RELAY_MEMBERSHIP=true \
    "RELAY_OWNER_PUBKEY=${PILOT_ENV[CORE_BANKER_PUBLIC_KEY]}" \
    "BUZZ_RELAY_PRIVATE_KEY=${PILOT_ENV[CORE_RELAY_PRIVATE_KEY]}" \
    BUZZ_GIT_ENABLED=false \
    "$relay_bin" > "$PILOT_STATE_DIR/relay.log" 2>&1 &
  relay_pid=$!
  marker_written=false
  for _ in $(seq 1 10); do
    if pilot_write_marker "$relay_marker" "$relay_pid" "$relay_bin"; then marker_written=true; break; fi
    sleep 0.05
  done
  [[ "$marker_written" == true ]] || { pilot_die 'relay exited during bootstrap'; exit 1; }
  for _ in $(seq 1 30); do relay_ready && break; sleep 1; done
  relay_ready || { pilot_stop_marker "$relay_marker" "$relay_bin"; pilot_die 'bootstrap relay did not become ready'; exit 1; }
fi

"${admin_env[@]}" "$PILOT_BIN_DIR/buzz-admin" add-member \
  --pubkey "${PILOT_ENV[CORE_AGENT_PUBLIC_KEY]}" --role member >> "$PILOT_STATE_DIR/bootstrap.log" 2>&1
"${admin_env[@]}" "$PILOT_BIN_DIR/buzz-admin" add-member \
  --pubkey "${PILOT_ENV[CORE_NON_OWNER_PUBLIC_KEY]}" --role member >> "$PILOT_STATE_DIR/bootstrap.log" 2>&1

buzz_as() {
  local private_key="$1"; shift
  env -i "PATH=$PILOT_BIN_DIR:/usr/bin:/bin" BUZZ_RELAY_URL=http://127.0.0.1:3000 \
    "BUZZ_PRIVATE_KEY=$private_key" "$PILOT_BIN_DIR/buzz" "$@"
}
buzz_as "${PILOT_ENV[CORE_BANKER_PRIVATE_KEY]}" users set-profile --name 'Core Banker' >/dev/null
buzz_as "${PILOT_ENV[CORE_AGENT_PRIVATE_KEY]}" users set-profile --name 'Core Research Partner' >/dev/null
buzz_as "${PILOT_ENV[CORE_NON_OWNER_PRIVATE_KEY]}" users set-profile --name 'Synthetic Non-Owner' >/dev/null

find_or_create_channel() {
  local name="$1" description="$2" matches count result
  matches="$(buzz_as "${PILOT_ENV[CORE_BANKER_PRIVATE_KEY]}" channels search --query "$name" --exact)"
  count="$(jq -er 'length' <<< "$matches")" || return 1
  case "$count" in
    0)
      result="$(buzz_as "${PILOT_ENV[CORE_BANKER_PRIVATE_KEY]}" channels create \
        --name "$name" --type stream --visibility private --description "$description")"
      jq -er 'select(.accepted == true) | .channel_id' <<< "$result"
      ;;
    1) jq -er '.[0].channel_id' <<< "$matches" ;;
    *) pilot_die "multiple exact channel matches for $name"; return 1 ;;
  esac
}

research_channel="$(find_or_create_channel core-research 'Core public/synthetic research pilot')"
second_channel="$(find_or_create_channel core-control 'Synthetic second-channel scope control')"
[[ "$research_channel" != "$second_channel" ]] || { pilot_die 'pilot channels must be distinct'; exit 1; }
for channel in "$research_channel" "$second_channel"; do
  buzz_as "${PILOT_ENV[CORE_BANKER_PRIVATE_KEY]}" channels add-member --channel "$channel" \
    --pubkey "${PILOT_ENV[CORE_AGENT_PUBLIC_KEY]}" --role bot >/dev/null
  buzz_as "${PILOT_ENV[CORE_BANKER_PRIVATE_KEY]}" channels add-member --channel "$channel" \
    --pubkey "${PILOT_ENV[CORE_NON_OWNER_PUBLIC_KEY]}" --role member >/dev/null
done

channels_tmp="$(mktemp "$PILOT_STATE_DIR/.channels.env.XXXXXX")"
printf 'CORE_RESEARCH_CHANNEL_ID=%s\nCORE_SECOND_CHANNEL_ID=%s\n' "$research_channel" "$second_channel" > "$channels_tmp"
chmod 600 "$channels_tmp"
mv "$channels_tmp" "$PILOT_CHANNELS_FILE"

config_parent="$(dirname "$PILOT_CONFIG_FILE")"
mkdir -p "$config_parent"; chmod 700 "$config_parent"
config_tmp="$(mktemp "$config_parent/.pilot.env.XXXXXX")"
sed -e "s/11111111-1111-4111-8111-111111111111/$research_channel/" \
  -e "s/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/${PILOT_ENV[CORE_BANKER_PUBLIC_KEY]}/" \
  "$PILOT_REPO_ROOT/config/core-pilot/core-pilot.env.example" > "$config_tmp"
chmod 600 "$config_tmp"
if [[ -e "$PILOT_CONFIG_FILE" ]]; then
  cmp -s "$config_tmp" "$PILOT_CONFIG_FILE" || { rm -f "$config_tmp"; pilot_die 'existing pilot config differs from generated safe config'; exit 1; }
  rm -f "$config_tmp"
else
  mv "$config_tmp" "$PILOT_CONFIG_FILE"
fi

printf 'Core pilot bootstrap is ready; ACP remains gated until the local OpenAI credential is configured.\n'
