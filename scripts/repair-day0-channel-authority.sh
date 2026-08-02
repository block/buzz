#!/usr/bin/env bash
# Normalize a durable manager identity across Steve's Day 0 Buzz pilot channels.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/repair-day0-channel-authority.sh \
    [--target-pubkey <64-char-hex>] \
    [--target-role <owner|admin>] \
    [--allow-local-fallback] \
    [--backup-dir <dir>] \
    [--skip-proof]

Normal path:
- If BUZZ_PRIVATE_KEY is set and is already authorized on the Day 0 channels,
  the helper first tries ordinary `buzz channels add-member`.

Fallback path:
- If the normal path cannot add the target manager and --allow-local-fallback
  is present, the helper creates a fresh local Postgres backup and injects the
  target manager as a relay member when needed and as an owner/admin on the
  missing Day 0 channels only.

Proof path:
- Unless --skip-proof is set, the helper requires BUZZ_PILOT_PROOF_PRIVATE_KEY
  or BUZZ_PRIVATE_KEY and verifies privileged access with
  `buzz channels update --no-ttl` on every Day 0 channel.

If --target-pubkey is omitted, the helper derives it from
BUZZ_PILOT_PROOF_PRIVATE_KEY or BUZZ_PRIVATE_KEY via `buzz-admin public-key`.
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

redact_sensitive() {
  local text="${1:-}"
  if [[ -n "${BUZZ_PRIVATE_KEY:-}" ]]; then
    text="${text//${BUZZ_PRIVATE_KEY}/<redacted-private-key>}"
  fi
  if [[ -n "${BUZZ_PILOT_PROOF_PRIVATE_KEY:-}" ]]; then
    text="${text//${BUZZ_PILOT_PROOF_PRIVATE_KEY}/<redacted-proof-private-key>}"
  fi
  printf '%s' "${text}"
}

validate_hex64() {
  local value="${1:-}"
  [[ "${value}" =~ ^[0-9a-f]{64}$ ]] || die "expected a 64-char lowercase hex pubkey, got '${value}'"
}

validate_target_role() {
  case "$1" in
    owner|admin) ;;
    *)
      die "--target-role must be owner or admin (got: $1)"
      ;;
  esac
}

csv_contains() {
  local csv="${1:-}"
  local needle="${2:-}"
  [[ ",${csv}," == *",${needle},"* ]]
}

resolve_buzz_cli() {
  local candidate="${BUZZ_PILOT_CLI:-}"
  if [[ -n "${candidate}" ]]; then
    [[ -x "${candidate}" ]] || die "Buzz CLI is not executable: ${candidate}"
    printf '%s' "${candidate}"
    return 0
  fi

  if [[ -x "${HELPER_ROOT}/scripts/buzz" ]]; then
    printf '%s' "${HELPER_ROOT}/scripts/buzz"
  elif [[ -x "${REPO_ROOT}/target/debug/buzz" ]]; then
    printf '%s' "${REPO_ROOT}/target/debug/buzz"
  elif [[ -x "${REPO_ROOT}/target/release/buzz" ]]; then
    printf '%s' "${REPO_ROOT}/target/release/buzz"
  elif [[ -x "${REPO_ROOT}/.hermit/rust/bin/buzz" ]]; then
    printf '%s' "${REPO_ROOT}/.hermit/rust/bin/buzz"
  else
    return 1
  fi
}

resolve_buzz_admin() {
  local candidate="${BUZZ_PILOT_ADMIN_CLI:-}"
  if [[ -n "${candidate}" ]]; then
    [[ -x "${candidate}" ]] || die "Buzz admin CLI is not executable: ${candidate}"
    "${candidate}" public-key --help >/dev/null 2>&1 || \
      die "Buzz admin CLI does not support public-key: ${candidate}"
    printf '%s' "${candidate}"
    return 0
  fi

  if [[ -x "${REPO_ROOT}/target/debug/buzz-admin" ]]; then
    candidate="${REPO_ROOT}/target/debug/buzz-admin"
  elif [[ -x "${REPO_ROOT}/target/release/buzz-admin" ]]; then
    candidate="${REPO_ROOT}/target/release/buzz-admin"
  else
    return 1
  fi

  if "${candidate}" public-key --help >/dev/null 2>&1; then
    printf '%s' "${candidate}"
  else
    return 1
  fi
}

resolve_cargo() {
  if command -v cargo >/dev/null 2>&1; then
    command -v cargo
  elif [[ -x "${REPO_ROOT}/bin/cargo" ]]; then
    printf '%s' "${REPO_ROOT}/bin/cargo"
  else
    return 1
  fi
}

derive_target_pubkey() {
  local proof_key output
  proof_key="${BUZZ_PILOT_PROOF_PRIVATE_KEY:-${BUZZ_PRIVATE_KEY:-}}"
  [[ -n "${proof_key}" ]] || die "--target-pubkey is required unless BUZZ_PILOT_PROOF_PRIVATE_KEY or BUZZ_PRIVATE_KEY is set"

  if [[ -n "${BUZZ_ADMIN_CLI:-}" ]]; then
    output="$(BUZZ_PRIVATE_KEY="${proof_key}" "${BUZZ_ADMIN_CLI}" public-key 2>&1)" || \
      die "failed to derive target pubkey with buzz-admin: $(redact_sensitive "${output}")"
  else
    [[ -n "${CARGO_BIN:-}" ]] || die "failed to derive target pubkey: cargo is unavailable; activate Hermit or set BUZZ_PILOT_ADMIN_CLI"
    output="$(BUZZ_PRIVATE_KEY="${proof_key}" "${CARGO_BIN}" run --quiet -p buzz-admin -- public-key 2>&1)" || \
      die "failed to derive target pubkey with cargo-run buzz-admin: $(redact_sensitive "${output}")"
  fi

  printf '%s' "${output}" | tail -n 1
}

run_psql() {
  local sql="$1"
  docker exec "${POSTGRES_CONTAINER}" \
    psql \
      -U "${POSTGRES_USER}" \
      -d "${POSTGRES_DB}" \
      -At \
      -F $'\t' \
      -v ON_ERROR_STOP=1 \
      -c "${sql}"
}

require_target_is_relay_member() {
  local sql role
  sql=$(cat <<EOF
-- day0-relay-member-check
SELECT rm.role
FROM communities c
JOIN relay_members rm ON rm.community_id = c.id
WHERE c.host = '${COMMUNITY_HOST}'
  AND rm.pubkey = '${target_pubkey}'
LIMIT 1;
EOF
)
  role="$(run_psql "${sql}")"
  if [[ -z "${role}" ]]; then
    if [[ "${allow_local_fallback}" -eq 1 ]]; then
      relay_member_missing=1
      return 0
    fi
    die "target pubkey ${target_pubkey} is not a relay member for ${COMMUNITY_HOST}; rerun with --allow-local-fallback to create a backup and repair local relay membership first"
  fi
}

load_day0_rows() {
  "${AUDIT_SCRIPT}" --format tsv
}

read_day0_rows_into_array() {
  local row
  day0_rows=()
  while IFS= read -r row; do
    day0_rows+=("${row}")
  done < <(load_day0_rows)
}

channel_meets_target_role() {
  local owners_csv="${1:-}"
  local admins_csv="${2:-}"

  if [[ "${target_role}" == "owner" ]]; then
    csv_contains "${owners_csv}" "${target_pubkey}"
  else
    csv_contains "${owners_csv}" "${target_pubkey}" || csv_contains "${admins_csv}" "${target_pubkey}"
  fi
}

collect_missing_channels() {
  local row name channel_id owners_csv admins_csv
  missing_channel_names=()
  missing_channel_ids=()

  for row in "$@"; do
    IFS=$'\t' read -r name channel_id _ _ _ owners_csv admins_csv <<<"${row}"
    if ! channel_meets_target_role "${owners_csv}" "${admins_csv}"; then
      missing_channel_names+=("${name}")
      missing_channel_ids+=("${channel_id}")
    fi
  done
}

attempt_normal_repair() {
  local i output status name channel_id

  if [[ -z "${BUZZ_PRIVATE_KEY:-}" ]]; then
    return 0
  fi
  if [[ -z "${BUZZ_CLI:-}" ]]; then
    return 0
  fi

  normal_path_attempted=1
  for ((i = 0; i < ${#missing_channel_ids[@]}; i += 1)); do
    name="${missing_channel_names[$i]}"
    channel_id="${missing_channel_ids[$i]}"

    set +e
    output="$(
      BUZZ_RELAY_URL="${RELAY_HTTP_URL}" \
      BUZZ_PRIVATE_KEY="${BUZZ_PRIVATE_KEY}" \
      "${BUZZ_CLI}" channels add-member \
        --channel "${channel_id}" \
        --pubkey "${target_pubkey}" \
        --role "${target_role}" 2>&1
    )"
    status=$?
    set -e

    if [[ "${status}" -ne 0 ]]; then
      normal_failures+=("${name}: $(redact_sensitive "${output}")")
    fi
  done
}

create_backup() {
  local backup_base backup_tmp
  if [[ -n "${backup_path}" ]]; then
    return 0
  fi

  mkdir -p -m 700 "${backup_dir}"
  chmod 700 "${backup_dir}"
  backup_base="$(mktemp "${backup_dir}/buzz-local-before-day0-authority-repair.XXXXXXXXXXXX")"
  backup_tmp="${backup_base}.dump"
  mv "${backup_base}" "${backup_tmp}"
  chmod 600 "${backup_tmp}"

  if ! docker exec "${POSTGRES_CONTAINER}" \
      pg_dump \
        -U "${POSTGRES_USER}" \
        -d "${POSTGRES_DB}" \
        -Fc > "${backup_tmp}"; then
    rm -f "${backup_tmp}"
    die "failed to create fallback backup at ${backup_tmp}"
  fi

  backup_path="${backup_tmp}"
}

apply_relay_member_fallback() {
  local sql

  create_backup
  fallback_applied=1
  relay_member_fallback_applied=1

  sql=$(cat <<EOF
-- day0-relay-member-upsert
INSERT INTO users (community_id, pubkey)
SELECT c.id, decode('${target_pubkey}', 'hex')
FROM communities c
WHERE c.host = '${COMMUNITY_HOST}'
ON CONFLICT (community_id, pubkey) DO NOTHING;

INSERT INTO relay_members (community_id, pubkey, role, added_by)
SELECT c.id, '${target_pubkey}', 'admin', NULL
FROM communities c
WHERE c.host = '${COMMUNITY_HOST}'
ON CONFLICT (community_id, pubkey)
DO UPDATE SET
  role = CASE
    WHEN relay_members.role = 'owner' THEN relay_members.role
    ELSE 'admin'
  END,
  updated_at = NOW();
EOF
)
  run_psql "${sql}" >/dev/null
  relay_member_missing=0
}

apply_local_fallback() {
  local i channel_id sql

  create_backup
  fallback_applied=1

  for ((i = 0; i < ${#missing_channel_ids[@]}; i += 1)); do
    channel_id="${missing_channel_ids[$i]}"
    sql=$(cat <<EOF
-- day0-channel-member-upsert
INSERT INTO channel_members (
  community_id,
  channel_id,
  pubkey,
  role,
  joined_at,
  invited_by,
  removed_at,
  removed_by,
  hidden_at
)
SELECT
  c.id,
  ch.id,
  decode('${target_pubkey}', 'hex'),
  '${target_role}'::member_role,
  NOW(),
  NULL,
  NULL,
  NULL,
  NULL
FROM communities c
JOIN channels ch ON ch.community_id = c.id
WHERE c.host = '${COMMUNITY_HOST}'
  AND ch.id = '${channel_id}'
ON CONFLICT (community_id, channel_id, pubkey)
DO UPDATE SET
  role = EXCLUDED.role,
  removed_at = NULL,
  removed_by = NULL,
  hidden_at = NULL;
EOF
)
    run_psql "${sql}" >/dev/null
  done
}

run_privileged_proof() {
  local proof_key channel_id output status

  if [[ "${skip_proof}" -eq 1 ]]; then
    return 0
  fi

  proof_key="${BUZZ_PILOT_PROOF_PRIVATE_KEY:-${BUZZ_PRIVATE_KEY:-}}"
  [[ -n "${proof_key}" ]] || die "proof requires BUZZ_PILOT_PROOF_PRIVATE_KEY or BUZZ_PRIVATE_KEY unless --skip-proof is used"
  [[ -n "${BUZZ_CLI:-}" ]] || die "proof requires a Buzz CLI; build it or set BUZZ_PILOT_CLI"

  for channel_id in "${day0_channel_ids[@]}"; do
    set +e
    output="$(
      BUZZ_RELAY_URL="${RELAY_HTTP_URL}" \
      BUZZ_PRIVATE_KEY="${proof_key}" \
      "${BUZZ_CLI}" channels update \
        --channel "${channel_id}" \
        --no-ttl 2>&1
    )"
    status=$?
    set -e

    if [[ "${status}" -ne 0 ]]; then
      die "privileged proof failed for ${channel_id}: $(redact_sensitive "${output}")"
    fi
    if ! printf '%s\n' "${output}" | grep -Fq '"accepted":true'; then
      die "privileged proof was not accepted for ${channel_id}: $(redact_sensitive "${output}")"
    fi
  done
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
HELPER_ROOT="$(cd "${REPO_ROOT}/.." && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-day0-channel-authority.sh"

[[ -x "${AUDIT_SCRIPT}" ]] || die "expected audit helper at ${AUDIT_SCRIPT}"

target_pubkey="${BUZZ_PILOT_DAY0_MANAGER_PUBKEY:-}"
target_role="${BUZZ_PILOT_DAY0_TARGET_ROLE:-admin}"
allow_local_fallback=0
skip_proof=0
backup_dir="${BUZZ_PILOT_BACKUP_DIR:-${HOME}/Backups/buzz}"
relay_member_missing=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target-pubkey)
      [[ $# -ge 2 ]] || die "--target-pubkey requires a value"
      target_pubkey="$2"
      shift 2
      ;;
    --target-role)
      [[ $# -ge 2 ]] || die "--target-role requires a value"
      target_role="$2"
      shift 2
      ;;
    --allow-local-fallback)
      allow_local_fallback=1
      shift
      ;;
    --backup-dir)
      [[ $# -ge 2 ]] || die "--backup-dir requires a value"
      backup_dir="$2"
      shift 2
      ;;
    --skip-proof)
      skip_proof=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

POSTGRES_CONTAINER="${BUZZ_PILOT_POSTGRES_CONTAINER:-buzz-postgres}"
POSTGRES_USER="${BUZZ_PILOT_POSTGRES_USER:-buzz}"
POSTGRES_DB="${BUZZ_PILOT_POSTGRES_DB:-buzz}"
COMMUNITY_HOST="${BUZZ_PILOT_COMMUNITY_HOST:-localhost:3030}"
RELAY_HTTP_URL="${BUZZ_PILOT_RELAY_HTTP_URL:-http://localhost:3030}"
BUZZ_CLI="$(resolve_buzz_cli || true)"
BUZZ_ADMIN_CLI="$(resolve_buzz_admin || true)"
CARGO_BIN="$(resolve_cargo || true)"

if [[ -z "${target_pubkey}" ]]; then
  target_pubkey="$(derive_target_pubkey)"
fi
validate_hex64 "${target_pubkey}"
validate_target_role "${target_role}"

require_target_is_relay_member

read_day0_rows_into_array
collect_missing_channels "${day0_rows[@]}"
day0_channel_ids=()
for row in "${day0_rows[@]}"; do
  IFS=$'\t' read -r _ channel_id _ _ _ _ _ <<<"${row}"
  day0_channel_ids+=("${channel_id}")
done

declare -a normal_failures=()
normal_path_attempted=0
fallback_applied=0
relay_member_fallback_applied=0
backup_path=""

if [[ "${relay_member_missing}" -eq 1 ]]; then
  apply_relay_member_fallback
fi

if [[ "${#missing_channel_ids[@]}" -gt 0 ]]; then
  attempt_normal_repair
  read_day0_rows_into_array
  collect_missing_channels "${day0_rows[@]}"
fi

if [[ "${#missing_channel_ids[@]}" -gt 0 ]]; then
  if [[ "${allow_local_fallback}" -ne 1 ]]; then
    if [[ "${normal_path_attempted}" -eq 1 && "${#normal_failures[@]}" -gt 0 ]]; then
      printf 'normal add-member path failed for:\n' >&2
      printf '  %s\n' "${normal_failures[@]}" >&2
    fi
    printf 'missing Day 0 authority remains for target %s on: %s\n' \
      "${target_pubkey}" \
      "$(IFS=,; echo "${missing_channel_names[*]}")" >&2
    printf 'rerun with --allow-local-fallback to create a fresh backup and apply the documented local-only repair path.\n' >&2
    exit 1
  fi

  apply_local_fallback
  read_day0_rows_into_array
  collect_missing_channels "${day0_rows[@]}"
fi

if [[ "${#missing_channel_ids[@]}" -gt 0 ]]; then
  die "target pubkey ${target_pubkey} is still missing required Day 0 authority after repair"
fi

run_privileged_proof

echo "Day 0 authority normalized for ${target_pubkey} (${target_role}) on ${COMMUNITY_HOST}."
if [[ "${normal_path_attempted}" -eq 1 ]]; then
  if [[ "${#normal_failures[@]}" -eq 0 ]]; then
    echo "Normal add-member path completed without recorded failures."
  else
    echo "Normal add-member path was attempted but did not fully repair every channel."
  fi
else
  echo "Normal add-member path was skipped because no current authorized Buzz write key was available."
fi

if [[ "${fallback_applied}" -eq 1 ]]; then
  echo "Local fallback applied after backup: ${backup_path}"
else
  echo "Local fallback was not needed."
fi

if [[ "${relay_member_fallback_applied}" -eq 1 ]]; then
  echo "Relay membership fallback added ${target_pubkey} as a community admin for ${COMMUNITY_HOST}."
fi

if [[ "${skip_proof}" -eq 1 ]]; then
  echo "Privileged proof was skipped by request."
else
  echo "Privileged proof succeeded with buzz channels update --no-ttl on all four Day 0 channels."
fi

"${AUDIT_SCRIPT}"
