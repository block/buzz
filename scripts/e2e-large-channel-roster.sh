#!/usr/bin/env bash
# Prove a channel member past the historical 1,000-row boundary is present in
# relay-served discovery, can write, and survives an authoritative republish.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

: "${DATABASE_URL:?set DATABASE_URL to the isolated relay database}"
: "${BUZZ_RELAY_URL:=http://localhost:3030}"
: "${RELAY_URL:=ws://localhost:3030}"
: "${BUZZ_RELAY_PRIVATE_KEY:=0000000000000000000000000000000000000000000000000000000000000001}"
export BUZZ_RELAY_URL RELAY_URL BUZZ_RELAY_PRIVATE_KEY
unset BUZZ_AUTH_TAG

for binary in buzz buzz-admin; do
  resolved="$(command -v "$binary" || true)"
  [[ "$resolved" == "$REPO_ROOT/target/release/$binary" ]] || {
    echo "error: $binary must resolve to $REPO_ROOT/target/release/$binary (got ${resolved:-not found})" >&2
    exit 1
  }
done
command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 1; }
command -v psql >/dev/null || { echo "error: psql is required" >&2; exit 1; }

key_field() {
  awk -v label="$1" '$1 == label && $2 == "key:" { print $3 }'
}

OWNER_GEN="$(buzz-admin generate-key)"
OWNER_SK="$(printf '%s\n' "$OWNER_GEN" | key_field Secret)"
export BUZZ_PRIVATE_KEY="$OWNER_SK"

CHANNEL="$(buzz channels create \
  --name "roster-boundary-$$" --type stream --visibility open | jq -er '.channel_id')"

LATE_GEN="$(buzz-admin generate-key)"
LATE_SK="$(printf '%s\n' "$LATE_GEN" | key_field Secret)"
LATE_PUBKEY="$(printf '%s\n' "$LATE_GEN" | key_field Public)"

# The creator is roster position 1. Add 1,499 fixtures followed by the real
# late identity at position 1,501. A final API-added member forces the relay to
# emit a fresh discovery snapshot through the normal membership-change path.
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  --set=channel="$CHANNEL" --set=late_pubkey="$LATE_PUBKEY" <<'SQL'
WITH target AS (
  SELECT community_id, id
  FROM channels
  WHERE id = :'channel'::uuid
), fixtures AS (
  SELECT n, decode(lpad(to_hex(n + 65536), 64, '0'), 'hex') AS pubkey
  FROM generate_series(1, 1499) AS n
)
INSERT INTO channel_members (community_id, channel_id, pubkey, role, joined_at)
SELECT target.community_id, target.id, fixtures.pubkey, 'member',
       NOW() + fixtures.n * interval '1 millisecond'
FROM target CROSS JOIN fixtures;

INSERT INTO channel_members (community_id, channel_id, pubkey, role, joined_at)
SELECT community_id, id, decode(:'late_pubkey', 'hex'), 'member',
       NOW() + interval '2 seconds'
FROM channels
WHERE id = :'channel'::uuid;
SQL

TRIGGER_GEN="$(buzz-admin generate-key)"
TRIGGER_PUBKEY="$(printf '%s\n' "$TRIGGER_GEN" | key_field Public)"
buzz channels add-member --channel "$CHANNEL" --pubkey "$TRIGGER_PUBKEY" --role member >/dev/null

BEFORE="$(buzz channels members --channel "$CHANNEL")"
BEFORE_COUNT="$(jq 'length' <<<"$BEFORE")"
jq -e --arg pk "$LATE_PUBKEY" 'any(.[]; .pubkey == $pk and .role == "member")' \
  <<<"$BEFORE" >/dev/null
(( BEFORE_COUNT > 1000 ))
printf 'PASS discovery-before-republish channel=%s members=%s late_pubkey=%s\n' \
  "$CHANNEL" "$BEFORE_COUNT" "$LATE_PUBKEY"

export BUZZ_PRIVATE_KEY="$LATE_SK"
ACTION="$(buzz messages send --channel "$CHANNEL" --content "member-1501-action")"
jq -e '.accepted == true' <<<"$ACTION" >/dev/null
ACTION_ID="$(jq -er '.event_id' <<<"$ACTION")"
buzz messages get --channel "$CHANNEL" --limit 10 \
  | jq -e --arg id "$ACTION_ID" 'any(.[]; .id == $id and .content == "member-1501-action")' >/dev/null
printf 'PASS late-member-action event_id=%s\n' "$ACTION_ID"

# Targeted roster repair must not replace canonical metadata or admin events.
# Preserve both the event ID and complete tags, including fields this backfill
# command does not know how to rebuild (DM participants, topic, TTL, etc.).
DISCOVERY_BEFORE="$(psql "$DATABASE_URL" -AtX -v ON_ERROR_STOP=1 \
  --set=channel="$CHANNEL" <<'SQL'
SELECT jsonb_object_agg(kind::text, jsonb_build_object(
  'id', encode(id, 'hex'),
  'tags', tags
) ORDER BY kind)::text
FROM events
WHERE channel_id = :'channel'::uuid
  AND kind IN (39000, 39001)
  AND deleted_at IS NULL;
SQL
)"
jq -e 'has("39000") and has("39001")' <<<"$DISCOVERY_BEFORE" >/dev/null

DATABASE_URL="$DATABASE_URL" RELAY_URL="$RELAY_URL" \
  buzz-admin reconcile-channels --channel "$CHANNEL" >/dev/null

DISCOVERY_AFTER="$(psql "$DATABASE_URL" -AtX -v ON_ERROR_STOP=1 \
  --set=channel="$CHANNEL" <<'SQL'
SELECT jsonb_object_agg(kind::text, jsonb_build_object(
  'id', encode(id, 'hex'),
  'tags', tags
) ORDER BY kind)::text
FROM events
WHERE channel_id = :'channel'::uuid
  AND kind IN (39000, 39001)
  AND deleted_at IS NULL;
SQL
)"
[[ "$DISCOVERY_AFTER" == "$DISCOVERY_BEFORE" ]]
printf 'PASS targeted-repair-preserves-metadata-and-admin-events channel=%s\n' "$CHANNEL"

AFTER="$(buzz channels members --channel "$CHANNEL")"
AFTER_COUNT="$(jq 'length' <<<"$AFTER")"
jq -e --arg pk "$LATE_PUBKEY" 'any(.[]; .pubkey == $pk and .role == "member")' \
  <<<"$AFTER" >/dev/null
(( AFTER_COUNT == BEFORE_COUNT ))
printf 'PASS discovery-after-republish channel=%s members=%s late_pubkey=%s\n' \
  "$CHANNEL" "$AFTER_COUNT" "$LATE_PUBKEY"
