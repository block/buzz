#!/usr/bin/env bash
# Export committed Core pilot code plus the minimum encrypted identity/channel
# state needed to rebuild a fresh local relay on another VM.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PILOT_REPO_ROOT="$(cd "$script_dir/.." && pwd)"
source "$script_dir/core-pilot-lib.sh"

output_dir=
base_revision="$(pilot_default_transfer_base_commit)"
PILOT_SECRETS_FILE="$(pilot_default_secrets_file)"
PILOT_STATE_DIR="$(pilot_default_state_dir)"
passphrase_fd=

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      [[ $# -ge 2 ]] || { pilot_die '--output requires a directory'; exit 1; }
      output_dir="$2"; shift 2
      ;;
    --base)
      [[ $# -ge 2 ]] || { pilot_die '--base requires a commit'; exit 1; }
      base_revision="$2"; shift 2
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

[[ -n "$output_dir" ]] || { pilot_die '--output is required'; exit 1; }
if [[ -n "$passphrase_fd" && ( ! "$passphrase_fd" =~ ^[0-9]+$ || "$passphrase_fd" -lt 3 ) ]]; then
  pilot_die 'passphrase descriptor must be an open descriptor numbered 3 or higher'
  exit 1
fi
for command_name in git gpg openssl realpath sha256sum stat xxd; do
  command -v "$command_name" >/dev/null 2>&1 || { pilot_die "$command_name is required for export"; exit 1; }
done

repo_canonical="$(realpath -e -- "$PILOT_REPO_ROOT" 2>/dev/null)" || { pilot_die 'unable to resolve repository root'; exit 1; }
git_root="$(git -C "$PILOT_REPO_ROOT" rev-parse --show-toplevel 2>/dev/null)" || { pilot_die 'export must run from a Git checkout'; exit 1; }
git_root="$(realpath -e -- "$git_root" 2>/dev/null)" || { pilot_die 'unable to resolve Git checkout'; exit 1; }
[[ "$git_root" == "$repo_canonical" ]] || { pilot_die 'pilot scripts must belong to the checkout being exported'; exit 1; }
tracked_status="$(git -C "$PILOT_REPO_ROOT" status --porcelain --untracked-files=no 2>/dev/null)" || {
  pilot_die 'unable to inspect tracked checkout state'; exit 1;
}
[[ -z "$tracked_status" ]] || {
  pilot_die 'tracked checkout changes must be committed before export'
  exit 1
}
for tracked_path in scripts/core-pilot-export.sh scripts/core-pilot-import.sh \
  scripts/core-pilot-lib.sh config/core-pilot/core-research-partner.md; do
  git -C "$PILOT_REPO_ROOT" ls-files --error-unmatch -- "$tracked_path" >/dev/null 2>&1 || {
    pilot_die 'portable workflow files must be committed before export'; exit 1;
  }
done

source_commit="$(git -C "$PILOT_REPO_ROOT" rev-parse --verify 'HEAD^{commit}' 2>/dev/null)" || {
  pilot_die 'unable to resolve source commit'; exit 1;
}
base_commit="$(git -C "$PILOT_REPO_ROOT" rev-parse --verify --end-of-options "${base_revision}^{commit}" 2>/dev/null)" || {
  pilot_die 'bundle prerequisite commit is unavailable'; exit 1;
}
git -C "$PILOT_REPO_ROOT" merge-base --is-ancestor "$base_commit" "$source_commit" 2>/dev/null || {
  pilot_die 'bundle prerequisite is not an ancestor of the source commit'; exit 1;
}
[[ "$base_commit" != "$source_commit" ]] || { pilot_die 'bundle prerequisite leaves no incremental commits'; exit 1; }

PILOT_CHANNELS_FILE="$PILOT_STATE_DIR/channels.env"
pilot_check_private_input_file "$PILOT_SECRETS_FILE" 'pilot identity file'
pilot_check_private_input_file "$PILOT_CHANNELS_FILE" 'pilot channel file'
pilot_check_path_outside_repo "$PILOT_SECRETS_FILE" "$repo_canonical" 'pilot identity file'
pilot_check_path_outside_repo "$PILOT_CHANNELS_FILE" "$repo_canonical" 'pilot channel file'

declare -gA PILOT_ENV=()
pilot_read_file "$PILOT_SECRETS_FILE" secret
for key in OPENAI_COMPAT_API_KEY $(pilot_transfer_identity_keys); do
  pilot_require "$key"
done
[[ ${#PILOT_ENV[@]} -eq 9 ]] || { pilot_die 'pilot identity file is incomplete'; exit 1; }

declare -gA PILOT_TRANSFER=()
for key in $(pilot_transfer_identity_keys); do
  PILOT_TRANSFER["$key"]="${PILOT_ENV[$key]}"
done
pilot_validate_transfer_identity_values
pilot_load_channels

prompt_file="$PILOT_REPO_ROOT/config/core-pilot/core-research-partner.md"
[[ -f "$prompt_file" && ! -L "$prompt_file" ]] || { pilot_die 'reviewed Core prompt is missing or unsafe'; exit 1; }
prompt_canonical="$(realpath -e -- "$prompt_file" 2>/dev/null)" || { pilot_die 'unable to resolve reviewed Core prompt'; exit 1; }
[[ "$prompt_canonical" == "$repo_canonical/config/core-pilot/core-research-partner.md" ]] || {
  pilot_die 'reviewed Core prompt path is unsafe'; exit 1;
}
prompt_hash_line="$(sha256sum -- "$prompt_canonical" 2>/dev/null)" || { pilot_die 'unable to hash reviewed Core prompt'; exit 1; }
prompt_hash="${prompt_hash_line%% *}"
[[ "$prompt_hash" =~ ^[0-9a-f]{64}$ ]] || { pilot_die 'reviewed Core prompt hash is malformed'; exit 1; }
[[ "$prompt_hash" == "$(pilot_reviewed_prompt_sha256)" ]] || {
  pilot_die 'Core prompt does not match the independently reviewed digest'; exit 1;
}

pilot_check_new_external_directory_path "$output_dir" "$repo_canonical"
umask 077
mkdir -- "$output_dir" || { pilot_die 'unable to create transfer directory'; exit 1; }
chmod 700 "$output_dir" || { rmdir -- "$output_dir" 2>/dev/null || true; pilot_die 'unable to secure transfer directory'; exit 1; }
complete=false
bundle_tmp="$output_dir/.core-pilot.bundle.tmp"
state_tmp="$output_dir/.core-pilot-state.gpg.tmp"
manifest_tmp="$output_dir/.SHA256SUMS.tmp"
cleanup_export() {
  if [[ "$complete" != true ]]; then
    rm -f -- "$bundle_tmp" "$state_tmp" "$manifest_tmp" \
      "$output_dir/core-pilot.bundle" "$output_dir/core-pilot-state.gpg" "$output_dir/SHA256SUMS"
    rmdir -- "$output_dir" 2>/dev/null || true
  fi
}
trap cleanup_export EXIT

git -C "$PILOT_REPO_ROOT" bundle create "$bundle_tmp" HEAD "^$base_commit" >/dev/null 2>&1 || {
  pilot_die 'unable to create incremental Git bundle'; exit 1;
}
chmod 600 "$bundle_tmp"
git -C "$PILOT_REPO_ROOT" bundle verify "$bundle_tmp" >/dev/null 2>&1 || {
  pilot_die 'created Git bundle failed verification'; exit 1;
}
[[ "$(pilot_bundle_prerequisite "$bundle_tmp" 2>/dev/null)" == "$base_commit" \
   && "$(pilot_bundle_head "$bundle_tmp" 2>/dev/null)" == "$source_commit" ]] || {
  pilot_die 'created Git bundle metadata is inconsistent'; exit 1;
}

write_transfer_state() {
  printf '%s\n' \
    'CORE_PILOT_TRANSFER_SCHEMA=1' \
    "CORE_PILOT_SOURCE_COMMIT=$source_commit" \
    "CORE_PILOT_BUNDLE_BASE=$base_commit" \
    "CORE_PILOT_PROMPT_SHA256=$prompt_hash" \
    "CORE_RELAY_PUBLIC_KEY=${PILOT_TRANSFER[CORE_RELAY_PUBLIC_KEY]}" \
    "CORE_RELAY_PRIVATE_KEY=${PILOT_TRANSFER[CORE_RELAY_PRIVATE_KEY]}" \
    "CORE_BANKER_PUBLIC_KEY=${PILOT_TRANSFER[CORE_BANKER_PUBLIC_KEY]}" \
    "CORE_BANKER_PRIVATE_KEY=${PILOT_TRANSFER[CORE_BANKER_PRIVATE_KEY]}" \
    "CORE_AGENT_PUBLIC_KEY=${PILOT_TRANSFER[CORE_AGENT_PUBLIC_KEY]}" \
    "CORE_AGENT_PRIVATE_KEY=${PILOT_TRANSFER[CORE_AGENT_PRIVATE_KEY]}" \
    "CORE_NON_OWNER_PUBLIC_KEY=${PILOT_TRANSFER[CORE_NON_OWNER_PUBLIC_KEY]}" \
    "CORE_NON_OWNER_PRIVATE_KEY=${PILOT_TRANSFER[CORE_NON_OWNER_PRIVATE_KEY]}" \
    "CORE_RESEARCH_CHANNEL_ID=${PILOT_CHANNELS[CORE_RESEARCH_CHANNEL_ID]}" \
    "CORE_SECOND_CHANNEL_ID=${PILOT_CHANNELS[CORE_SECOND_CHANNEL_ID]}"
}

gpg_args=(--no-options --quiet --cipher-algo AES256)
if [[ -n "$passphrase_fd" ]]; then
  gpg_args+=(--batch --pinentry-mode loopback --passphrase-fd "$passphrase_fd")
fi
if write_transfer_state | gpg "${gpg_args[@]}" --symmetric --output "$state_tmp"; then
  gpg_status=0
else
  gpg_status=$?
fi
if [[ -n "${passphrase_fd:-}" ]]; then
  exec {passphrase_fd}<&-
fi
if [[ $gpg_status -ne 0 ]]; then
  pilot_die 'unable to encrypt private pilot state'
  exit 1
fi
[[ -s "$state_tmp" && -f "$state_tmp" && ! -L "$state_tmp" ]] || {
  pilot_die 'GPG did not create a valid encrypted state artifact'; exit 1;
}
chmod 600 "$state_tmp"
mv -- "$bundle_tmp" "$output_dir/core-pilot.bundle"
mv -- "$state_tmp" "$output_dir/core-pilot-state.gpg"
(
  cd "$output_dir"
  sha256sum -- core-pilot.bundle core-pilot-state.gpg > "$manifest_tmp"
) || { pilot_die 'unable to create transfer checksum manifest'; exit 1; }
chmod 600 "$manifest_tmp"
mv -- "$manifest_tmp" "$output_dir/SHA256SUMS"
complete=true
trap - EXIT
bundle_sha256="$(sha256sum -- "$output_dir/core-pilot.bundle" | awk '{print $1}')"
printf '%s\n' \
  'Core pilot transfer created; copy the private directory securely to the destination VM.' \
  "Expected source commit (record separately): $source_commit" \
  "Expected bundle SHA-256 (record separately): $bundle_sha256"
