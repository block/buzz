#!/usr/bin/env bash
# Validate a Core pilot transfer against this exact checkout, then restore only
# stable identities and channel UUIDs. Code is fetched from the bundle first;
# see docs/core-pilot-runbook.md for the clean-VM sequence.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PILOT_REPO_ROOT="$(cd "$script_dir/.." && pwd)"
source "$script_dir/core-pilot-lib.sh"

source_dir=
PILOT_SECRETS_FILE="$(pilot_default_secrets_file)"
PILOT_STATE_DIR="$(pilot_default_state_dir)"
passphrase_fd=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --source)
      [[ $# -ge 2 ]] || { pilot_die '--source requires a directory'; exit 1; }
      source_dir="$2"; shift 2
      ;;
    --secrets)
      [[ $# -ge 2 ]] || { pilot_die '--secrets requires a file'; exit 1; }
      PILOT_SECRETS_FILE="$2"; shift 2
      ;;
    --state-dir)
      [[ $# -ge 2 ]] || { pilot_die '--state-dir requires a directory'; exit 1; }
      PILOT_STATE_DIR="$2"; shift 2
      ;;
    --passphrase-fd)
      [[ $# -ge 2 ]] || { pilot_die '--passphrase-fd requires a descriptor'; exit 1; }
      passphrase_fd="$2"; shift 2
      ;;
    *) pilot_die "unknown option: $1"; exit 1 ;;
  esac
done

[[ -n "$source_dir" ]] || { pilot_die '--source is required'; exit 1; }
if [[ -n "$passphrase_fd" && ( ! "$passphrase_fd" =~ ^[0-9]+$ || "$passphrase_fd" -lt 3 ) ]]; then
  pilot_die 'passphrase descriptor must be an open descriptor numbered 3 or higher'
  exit 1
fi
for command_name in git gpg openssl realpath sha256sum stat xxd; do
  command -v "$command_name" >/dev/null 2>&1 || { pilot_die "$command_name is required for import"; exit 1; }
done

repo_canonical="$(realpath -e -- "$PILOT_REPO_ROOT" 2>/dev/null)" || { pilot_die 'unable to resolve repository root'; exit 1; }
git_root="$(git -C "$PILOT_REPO_ROOT" rev-parse --show-toplevel 2>/dev/null)" || { pilot_die 'import must run from a Git checkout'; exit 1; }
git_root="$(realpath -e -- "$git_root" 2>/dev/null)" || { pilot_die 'unable to resolve Git checkout'; exit 1; }
[[ "$git_root" == "$repo_canonical" ]] || { pilot_die 'pilot scripts must belong to the checkout being imported'; exit 1; }
tracked_status="$(git -C "$PILOT_REPO_ROOT" status --porcelain --untracked-files=no 2>/dev/null)" || {
  pilot_die 'unable to inspect tracked checkout state'; exit 1;
}
[[ -z "$tracked_status" ]] || {
  pilot_die 'tracked checkout changes are not allowed during import'
  exit 1
}
for tracked_path in scripts/core-pilot-export.sh scripts/core-pilot-import.sh \
  scripts/core-pilot-lib.sh config/core-pilot/core-research-partner.md; do
  git -C "$PILOT_REPO_ROOT" ls-files --error-unmatch -- "$tracked_path" >/dev/null 2>&1 || {
    pilot_die 'portable workflow files are missing from the destination commit'; exit 1;
  }
done

pilot_check_private_directory "$source_dir" 'transfer source directory'
case "$source_dir" in
  "$repo_canonical"|"$repo_canonical"/*)
    pilot_die 'transfer source directory must live outside the repository'
    exit 1
    ;;
esac
bundle_file="$source_dir/core-pilot.bundle"
encrypted_file="$source_dir/core-pilot-state.gpg"
manifest_file="$source_dir/SHA256SUMS"
pilot_check_private_input_file "$bundle_file" 'Git bundle artifact'
pilot_check_private_input_file "$encrypted_file" 'encrypted state artifact'
pilot_check_private_input_file "$manifest_file" 'transfer checksum manifest'
pilot_verify_transfer_manifest "$source_dir" "$manifest_file"

umask 077
temporary_parent="${TMPDIR:-/tmp}"
pilot_check_temporary_parent "$temporary_parent"
temporary_dir=
decrypted_file=
identity_stage=
channels_stage=
cleanup_import() {
  if [[ -n "$decrypted_file" ]]; then rm -f -- "$decrypted_file"; fi
  if [[ -n "$temporary_dir" ]]; then
    rm -f -- "$temporary_dir/agent.env" "$temporary_dir/channels.env"
  fi
  if [[ -n "$identity_stage" ]]; then rm -f -- "$identity_stage"; fi
  if [[ -n "$channels_stage" ]]; then rm -f -- "$channels_stage"; fi
  if [[ -n "$temporary_dir" ]]; then rmdir -- "$temporary_dir" 2>/dev/null || true; fi
}
temporary_dir="$(mktemp -d "$temporary_parent/core-pilot-import.XXXXXX")" || {
  pilot_die 'unable to create private import workspace'; exit 1;
}
trap cleanup_import EXIT
chmod 700 "$temporary_dir" || { pilot_die 'unable to secure private import workspace'; exit 1; }
decrypted_file="$temporary_dir/state.env"

gpg_args=(--no-options --quiet)
if [[ -n "$passphrase_fd" ]]; then
  gpg_args+=(--batch --pinentry-mode loopback --passphrase-fd "$passphrase_fd")
fi
if gpg "${gpg_args[@]}" --output "$decrypted_file" --decrypt "$encrypted_file"; then
  gpg_status=0
else
  gpg_status=$?
fi
if [[ -n "${passphrase_fd:-}" ]]; then
  exec {passphrase_fd}<&-
fi
if [[ $gpg_status -ne 0 ]]; then
  pilot_die 'unable to decrypt private pilot state'
  exit 1
fi
chmod 600 "$decrypted_file"
pilot_read_transfer_file "$decrypted_file"

source_commit="${PILOT_TRANSFER[CORE_PILOT_SOURCE_COMMIT]}"
base_commit="${PILOT_TRANSFER[CORE_PILOT_BUNDLE_BASE]}"
current_commit="$(git -C "$PILOT_REPO_ROOT" rev-parse --verify 'HEAD^{commit}' 2>/dev/null)" || {
  pilot_die 'unable to resolve destination source commit'; exit 1;
}
[[ "$current_commit" == "$source_commit" ]] || {
  pilot_die 'destination checkout does not match the exported source commit'
  exit 1
}
git -C "$PILOT_REPO_ROOT" cat-file -e "$base_commit^{commit}" 2>/dev/null || {
  pilot_die 'bundle prerequisite commit is unavailable in the destination checkout'; exit 1;
}
git -C "$PILOT_REPO_ROOT" merge-base --is-ancestor "$base_commit" "$source_commit" 2>/dev/null || {
  pilot_die 'portable source commit does not descend from its bundle prerequisite'; exit 1;
}
git -C "$PILOT_REPO_ROOT" bundle verify "$bundle_file" >/dev/null 2>&1 || {
  pilot_die 'incremental Git bundle failed verification'; exit 1;
}
[[ "$(pilot_bundle_prerequisite "$bundle_file" 2>/dev/null)" == "$base_commit" \
   && "$(pilot_bundle_head "$bundle_file" 2>/dev/null)" == "$source_commit" ]] || {
  pilot_die 'incremental Git bundle does not match encrypted transfer metadata'; exit 1;
}

prompt_file="$PILOT_REPO_ROOT/config/core-pilot/core-research-partner.md"
[[ -f "$prompt_file" && ! -L "$prompt_file" ]] || { pilot_die 'reviewed Core prompt is missing or unsafe'; exit 1; }
prompt_canonical="$(realpath -e -- "$prompt_file" 2>/dev/null)" || { pilot_die 'unable to resolve reviewed Core prompt'; exit 1; }
[[ "$prompt_canonical" == "$repo_canonical/config/core-pilot/core-research-partner.md" ]] || {
  pilot_die 'reviewed Core prompt path is unsafe'; exit 1;
}
prompt_hash_line="$(sha256sum -- "$prompt_canonical" 2>/dev/null)" || { pilot_die 'unable to hash reviewed Core prompt'; exit 1; }
reviewed_prompt_hash="$(pilot_reviewed_prompt_sha256)"
[[ "${PILOT_TRANSFER[CORE_PILOT_PROMPT_SHA256]}" == "$reviewed_prompt_hash" \
   && "${prompt_hash_line%% *}" == "$reviewed_prompt_hash" ]] || {
  pilot_die 'reviewed Core prompt does not match the exported prompt hash'; exit 1;
}

printf '%s\n' \
  'OPENAI_COMPAT_API_KEY=' \
  "CORE_RELAY_PUBLIC_KEY=${PILOT_TRANSFER[CORE_RELAY_PUBLIC_KEY]}" \
  "CORE_RELAY_PRIVATE_KEY=${PILOT_TRANSFER[CORE_RELAY_PRIVATE_KEY]}" \
  "CORE_BANKER_PUBLIC_KEY=${PILOT_TRANSFER[CORE_BANKER_PUBLIC_KEY]}" \
  "CORE_BANKER_PRIVATE_KEY=${PILOT_TRANSFER[CORE_BANKER_PRIVATE_KEY]}" \
  "CORE_AGENT_PUBLIC_KEY=${PILOT_TRANSFER[CORE_AGENT_PUBLIC_KEY]}" \
  "CORE_AGENT_PRIVATE_KEY=${PILOT_TRANSFER[CORE_AGENT_PRIVATE_KEY]}" \
  "CORE_NON_OWNER_PUBLIC_KEY=${PILOT_TRANSFER[CORE_NON_OWNER_PUBLIC_KEY]}" \
  "CORE_NON_OWNER_PRIVATE_KEY=${PILOT_TRANSFER[CORE_NON_OWNER_PRIVATE_KEY]}" \
  > "$temporary_dir/agent.env"
printf '%s\n' \
  "CORE_RESEARCH_CHANNEL_ID=${PILOT_TRANSFER[CORE_RESEARCH_CHANNEL_ID]}" \
  "CORE_SECOND_CHANNEL_ID=${PILOT_TRANSFER[CORE_SECOND_CHANNEL_ID]}" \
  > "$temporary_dir/channels.env"
chmod 600 "$temporary_dir/agent.env" "$temporary_dir/channels.env"

secrets_parent="$(dirname -- "$PILOT_SECRETS_FILE")"
pilot_prepare_private_destination_directory "$secrets_parent" 'pilot secret directory' "$repo_canonical"
pilot_prepare_private_destination_directory "$PILOT_STATE_DIR" 'pilot state directory' "$repo_canonical"
PILOT_CHANNELS_FILE="$PILOT_STATE_DIR/channels.env"
[[ "$secrets_parent/$(basename -- "$PILOT_SECRETS_FILE")" == "$PILOT_SECRETS_FILE" ]] || {
  pilot_die 'pilot identity path must be canonical'; exit 1;
}
[[ "$PILOT_SECRETS_FILE" != "$PILOT_CHANNELS_FILE" ]] || {
  pilot_die 'pilot identity and channel destinations must be distinct'; exit 1;
}
pilot_check_existing_destination_file "$PILOT_SECRETS_FILE" 'pilot identity destination'
pilot_check_existing_destination_file "$PILOT_CHANNELS_FILE" 'pilot channel destination'
[[ ! -e "$secrets_parent/.agent.env.import" && ! -L "$secrets_parent/.agent.env.import" \
   && ! -e "$PILOT_STATE_DIR/.channels.env.import" && ! -L "$PILOT_STATE_DIR/.channels.env.import" ]] || {
  pilot_die 'unsafe legacy import staging path exists'
  exit 1
}
if [[ -e "$PILOT_SECRETS_FILE" ]] && ! cmp -s -- "$temporary_dir/agent.env" "$PILOT_SECRETS_FILE"; then
  pilot_die 'existing pilot identity state differs; refusing to overwrite it'
  exit 1
fi
if [[ -e "$PILOT_CHANNELS_FILE" ]] && ! cmp -s -- "$temporary_dir/channels.env" "$PILOT_CHANNELS_FILE"; then
  pilot_die 'existing pilot channel state differs; refusing to overwrite it'
  exit 1
fi

installed_secrets=false
installed_channels=false
rollback_new_state() {
  if [[ "$installed_channels" == true ]]; then rm -f -- "$PILOT_CHANNELS_FILE"; fi
  if [[ "$installed_secrets" == true ]]; then rm -f -- "$PILOT_SECRETS_FILE"; fi
  cleanup_import
}
trap rollback_new_state EXIT
if [[ ! -e "$PILOT_SECRETS_FILE" ]]; then
  identity_stage="$(mktemp "$secrets_parent/.agent.env.import.XXXXXX")" || {
    pilot_die 'unable to stage imported identity state'; exit 1;
  }
  cp -- "$temporary_dir/agent.env" "$identity_stage"
  chmod 600 "$identity_stage"
  ln -- "$identity_stage" "$PILOT_SECRETS_FILE" || {
    pilot_die 'pilot identity destination changed during import'; exit 1;
  }
  rm -f -- "$identity_stage"
  identity_stage=
  installed_secrets=true
fi
if [[ ! -e "$PILOT_CHANNELS_FILE" ]]; then
  channels_stage="$(mktemp "$PILOT_STATE_DIR/.channels.env.import.XXXXXX")" || {
    pilot_die 'unable to stage imported channel state'; exit 1;
  }
  cp -- "$temporary_dir/channels.env" "$channels_stage"
  chmod 600 "$channels_stage"
  ln -- "$channels_stage" "$PILOT_CHANNELS_FILE" || {
    pilot_die 'pilot channel destination changed during import'; exit 1;
  }
  rm -f -- "$channels_stage"
  channels_stage=
  installed_channels=true
fi
chmod 600 -- "$PILOT_SECRETS_FILE" "$PILOT_CHANNELS_FILE"
installed_secrets=false
installed_channels=false
trap cleanup_import EXIT
printf 'Core pilot identity and channel state imported; the OpenAI credential remains empty.\n'
