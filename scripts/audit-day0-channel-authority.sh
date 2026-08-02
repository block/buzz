#!/usr/bin/env bash
# Read-only audit of Steve's Day 0 Buzz pilot channel authority state.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/audit-day0-channel-authority.sh [--format table|tsv]

Reports the Day 0 pilot channels on localhost:3030, including current owner and
admin memberships, TTL posture, and archive posture.
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

format_member_list() {
  local members="${1:-}"
  if [[ -z "${members}" ]]; then
    printf '%s' "(none)"
  else
    printf '%s' "${members}"
  fi
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

fetch_day0_rows() {
  local sql
  sql=$(cat <<EOF
-- day0-authority-audit
SELECT
  ch.name,
  ch.id::text,
  encode(ch.created_by, 'hex') AS created_by_hex,
  COALESCE(ch.ttl_seconds::text, 'permanent') AS ttl_state,
  CASE WHEN ch.archived_at IS NULL THEN 'active' ELSE 'archived' END AS archive_state,
  COALESCE((
    SELECT string_agg(encode(cm.pubkey, 'hex'), ',' ORDER BY encode(cm.pubkey, 'hex'))
    FROM channel_members cm
    WHERE cm.community_id = ch.community_id
      AND cm.channel_id = ch.id
      AND cm.role = 'owner'::member_role
      AND cm.removed_at IS NULL
  ), '') AS owner_pubkeys,
  COALESCE((
    SELECT string_agg(encode(cm.pubkey, 'hex'), ',' ORDER BY encode(cm.pubkey, 'hex'))
    FROM channel_members cm
    WHERE cm.community_id = ch.community_id
      AND cm.channel_id = ch.id
      AND cm.role = 'admin'::member_role
      AND cm.removed_at IS NULL
  ), '') AS admin_pubkeys
FROM communities c
JOIN channels ch ON ch.community_id = c.id
WHERE c.host = '${COMMUNITY_HOST}'
  AND ch.name IN ('agent-runs', 'buzz-pilot', 'install-support', 'repo-review')
ORDER BY ch.name;
EOF
)
  run_psql "${sql}"
}

output_format="table"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --format)
      [[ $# -ge 2 ]] || die "--format requires a value"
      output_format="$2"
      shift 2
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

case "${output_format}" in
  table|tsv) ;;
  *)
    die "--format must be table or tsv (got: ${output_format})"
    ;;
esac

POSTGRES_CONTAINER="${BUZZ_PILOT_POSTGRES_CONTAINER:-buzz-postgres}"
POSTGRES_USER="${BUZZ_PILOT_POSTGRES_USER:-buzz}"
POSTGRES_DB="${BUZZ_PILOT_POSTGRES_DB:-buzz}"
COMMUNITY_HOST="${BUZZ_PILOT_COMMUNITY_HOST:-localhost:3030}"

declare -a day0_order=(
  "agent-runs"
  "buzz-pilot"
  "install-support"
  "repo-review"
)

declare -a expected_ids=(
  "d0bf00d9-e76d-44a8-bf4c-61725f79f3d4"
  "3cdf4550-0501-4825-b54e-87213ea08b66"
  "7cf15a6f-a601-4c40-92a3-5fee69594992"
  "577ef732-7ee7-44dd-bd3d-f2ef0473a286"
)

rows=()
while IFS= read -r row; do
  rows+=("${row}")
done < <(fetch_day0_rows)

if [[ "${#rows[@]}" -eq 0 ]]; then
  die "no Day 0 channels were visible for ${COMMUNITY_HOST}; verify the active pilot database first"
fi

if [[ "${#rows[@]}" -ne "${#day0_order[@]}" ]]; then
  die "expected ${#day0_order[@]} Day 0 channels for ${COMMUNITY_HOST}, found ${#rows[@]}"
fi

for i in "${!day0_order[@]}"; do
  expected_name="${day0_order[$i]}"
  expected_id="${expected_ids[$i]}"
  row="${rows[$i]}"
  IFS=$'\t' read -r name channel_id created_by_hex ttl_state archive_state owner_pubkeys admin_pubkeys <<<"${row}"

  if [[ -z "${name}" || -z "${channel_id}" ]]; then
    die "encountered an incomplete Day 0 authority row"
  fi
  if [[ "${name}" != "${expected_name}" ]]; then
    die "expected Day 0 channel '${expected_name}' in slot $((i + 1)), found '${name}'"
  fi
  if [[ "${channel_id}" != "${expected_id}" ]]; then
    die "channel '${name}' expected id ${expected_id}, found ${channel_id}"
  fi
done

if [[ "${output_format}" == "tsv" ]]; then
  for row in "${rows[@]}"; do
    printf '%s\n' "${row}"
  done
  exit 0
fi

echo "Day 0 authority audit for ${COMMUNITY_HOST}"
printf '%-16s %-36s %-9s %-9s %-64s %-64s\n' \
  "CHANNEL" \
  "CHANNEL ID" \
  "TTL" \
  "STATE" \
  "OWNERS" \
  "ADMINS"

for i in "${!day0_order[@]}"; do
  name="${day0_order[$i]}"
  IFS=$'\t' read -r _ channel_id _ ttl_state archive_state owner_pubkeys admin_pubkeys <<<"${rows[$i]}"
  printf '%-16s %-36s %-9s %-9s %-64s %-64s\n' \
    "${name}" \
    "${channel_id}" \
    "${ttl_state}" \
    "${archive_state}" \
    "$(format_member_list "${owner_pubkeys}")" \
    "$(format_member_list "${admin_pubkeys}")"
done
