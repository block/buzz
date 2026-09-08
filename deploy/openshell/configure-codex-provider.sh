#!/usr/bin/env bash

set -euo pipefail

readonly script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly profile_file="$script_dir/providers/codex-buzz.yaml"
readonly profile_id=codex-buzz
readonly codex_oauth_client_id=app_EMoamEEZ73f0CkXaXp7hrann

provider_name=codex-buzz
auth_file="${CODEX_HOME:-${HOME:?HOME is required}/.codex}/auth.json"
gateway=""
reset_refresh=0

usage() {
  cat <<'EOF'
Usage: configure-codex-provider.sh [options]

Create or reconcile an OpenShell provider whose gateway owns Codex OAuth
access-token refresh. The auth file is read once as bootstrap material; it is
never uploaded to a sandbox.

Options:
  --provider NAME   Provider instance name (default: codex-buzz)
  --auth-file PATH  Codex auth file (default: $CODEX_HOME/auth.json or ~/.codex/auth.json)
  --gateway NAME    OpenShell gateway name (default: active gateway)
  --reset-refresh   Replace gateway refresh state from the supplied auth file
  -h, --help        Show this help
EOF
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

while (($#)); do
  case "$1" in
    --provider)
      (($# >= 2)) || fail "--provider requires a value"
      provider_name=$2
      shift 2
      ;;
    --auth-file)
      (($# >= 2)) || fail "--auth-file requires a value"
      auth_file=$2
      shift 2
      ;;
    --gateway)
      (($# >= 2)) || fail "--gateway requires a value"
      gateway=$2
      shift 2
      ;;
    --reset-refresh)
      reset_refresh=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *) fail "unknown argument: $1" ;;
  esac
done

command -v jq >/dev/null || fail "jq is required"
command -v openshell >/dev/null || fail "openshell is required"
[[ -f "$auth_file" ]] || fail "Codex auth file not found: $auth_file"
[[ ! -L "$auth_file" ]] || fail "refusing symlinked Codex auth file: $auth_file"

openshell_cmd() {
  if [[ -n "$gateway" ]]; then
    openshell --gateway "$gateway" "$@"
  else
    openshell "$@"
  fi
}

import_or_update_profile() {
  local exported resource_version temporary_dir temporary_profile
  if ! exported="$(openshell_cmd provider profile export --output json "$profile_id" 2>/dev/null)"; then
    openshell_cmd provider profile lint --file "$profile_file" >/dev/null
    openshell_cmd provider profile import --file "$profile_file" >/dev/null
    return
  fi

  resource_version="$(jq -er '.resource_version | select(type == "number" and . > 0)' <<<"$exported")" \
    || fail "existing $profile_id profile has no resource version"
  temporary_dir="$(mktemp -d "${TMPDIR:-/tmp}/buzz-codex-profile.XXXXXX")"
  temporary_profile="$temporary_dir/profile.yaml"
  trap 'rm -rf "${temporary_dir:-}"' RETURN
  awk -v version="$resource_version" \
    '{ print; if (!inserted && $0 ~ /^id:/) { print "resource_version: " version; inserted=1 } }' \
    "$profile_file" >"$temporary_profile"
  openshell_cmd provider profile update "$profile_id" --file "$temporary_profile" >/dev/null
  rm -f "$temporary_profile"
  rmdir "$temporary_dir"
  trap - RETURN
}

access_token="$(jq -er '.tokens.access_token | select(type == "string" and length > 0)' "$auth_file")" \
  || fail "Codex auth file has no access token; run a fresh Codex login"
refresh_token="$(jq -er '.tokens.refresh_token | select(type == "string" and length > 0)' "$auth_file")" \
  || fail "Codex auth file has no refresh token; use ChatGPT login, not API-key login"
account_id="$(jq -er '.tokens.account_id | select(type == "string" and length > 0)' "$auth_file")" \
  || fail "Codex auth file has no account ID; run a fresh Codex login"
export CODEX_AUTH_ACCESS_TOKEN="$access_token"
export CODEX_AUTH_ACCOUNT_ID="$account_id"
export BUZZ_CODEX_REFRESH_TOKEN="$refresh_token"

import_or_update_profile

provider_exists=0
if openshell_cmd provider get "$provider_name" >/dev/null 2>&1; then
  provider_exists=1
fi

refresh_exists=0
if ((provider_exists)); then
  if refresh_status="$(openshell_cmd provider refresh status "$provider_name" \
    --credential-key CODEX_AUTH_ACCESS_TOKEN 2>&1)"; then
    [[ "$refresh_status" == *"No refresh configuration found"* ]] || refresh_exists=1
  elif [[ "$refresh_status" != *"No refresh configuration found"* ]]; then
    printf '%s\n' "$refresh_status" >&2
    fail "could not inspect provider refresh state"
  fi
fi

if ((reset_refresh && refresh_exists)); then
  openshell_cmd provider refresh delete "$provider_name" \
    --credential-key CODEX_AUTH_ACCESS_TOKEN >/dev/null
  refresh_exists=0
fi

if ((!provider_exists)); then
  openshell_cmd provider create --name "$provider_name" --type "$profile_id" \
    --credential CODEX_AUTH_ACCESS_TOKEN \
    --credential CODEX_AUTH_ACCOUNT_ID >/dev/null
elif ((!refresh_exists)); then
  openshell_cmd provider update "$provider_name" \
    --credential CODEX_AUTH_ACCESS_TOKEN \
    --credential CODEX_AUTH_ACCOUNT_ID >/dev/null
fi

if ((!refresh_exists)); then
  openshell_cmd provider refresh configure "$provider_name" \
    --credential-key CODEX_AUTH_ACCESS_TOKEN \
    --strategy oauth2-refresh-token \
    --material "client_id=$codex_oauth_client_id" \
    --secret-material-env refresh_token=BUZZ_CODEX_REFRESH_TOKEN >/dev/null
fi

if ! openshell_cmd provider refresh rotate "$provider_name" \
  --credential-key CODEX_AUTH_ACCESS_TOKEN >/dev/null; then
  fail "gateway rotation failed; obtain a fresh dedicated login and retry with --reset-refresh"
fi

printf "Codex provider '%s' is configured with gateway-managed refresh.\n" "$provider_name"
openshell_cmd provider refresh status "$provider_name" \
  --credential-key CODEX_AUTH_ACCESS_TOKEN
