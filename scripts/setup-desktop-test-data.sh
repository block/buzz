#!/usr/bin/env bash

set -euo pipefail

DB_HOST="${BUZZ_DB_HOST:-127.0.0.1}"
DB_PORT="${BUZZ_DB_PORT:-5432}"
DB_USER="${BUZZ_DB_USER:-buzz}"
DB_PASS="${BUZZ_DB_PASS:-buzz_dev}"
DB_NAME="${BUZZ_DB_NAME:-buzz}"

SYSTEM_PUBKEY="0000000000000000000000000000000000000000000000000000000000000000"
ALICE_PUBKEY="953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f"
BOB_PUBKEY="bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260"
CHARLIE_PUBKEY="554cef57437abac34522ac2c9f0490d685b72c80478cf9f7ed6f9570ee8624ea"
TYLER_PUBKEY="e5ebc6cdb579be112e336cc319b5989b4bb6af11786ea90dbe52b5f08d741b34"
AGENT_PUBKEY="db0b028cd36f4d3e36c8300cce87252c1f7fc9495ffecc53f393fcac341ffd36"

if command -v psql >/dev/null 2>&1; then
  run_psql() { PGPASSWORD="$DB_PASS" psql -h"$DB_HOST" -p"$DB_PORT" -U"$DB_USER" -d"$DB_NAME" -qtA "$@"; }
elif docker exec buzz-postgres psql --version >/dev/null 2>&1; then
  run_psql() {
    docker exec -e PGPASSWORD="$DB_PASS" buzz-postgres \
      psql -U"$DB_USER" -d"$DB_NAME" -qtA "$@"
  }
else
  echo "No psql client available. Start docker compose or install postgresql-client." >&2
  exit 1
fi

run_sql() {
  run_psql -c "$1"
}

uuid5_hex() {
  local slug="$1"
  python3 - "$slug" <<'PYEOF'
import sys, uuid
# Format as UUID with hyphens for Postgres
print(str(uuid.uuid5(uuid.NAMESPACE_DNS, sys.argv[1])))
PYEOF
}

echo "Checking database connection..."
run_sql "SELECT 1" >/dev/null

# Multi-tenant: every scoped row carries community_id. The relay resolves the
# tenant per-connection from the durable communities host map (WHERE host = $1
# against the *normalized* host). normalize_host() keeps non-default ports, so
# the host must be 'localhost:3000' verbatim to match RELAY_URL=ws://localhost:3000.
# The relay does not auto-seed communities (ensure_configured_community has no
# callers); this seed is the only writer of the dev community row. Insert it
# first so the channel/member INSERTs below satisfy their community_id FKs, and
# so the channel reconciler (BUZZ_RECONCILE_CHANNELS, which binds localhost:3000
# fail-closed and retries every 5s for 2min) can resolve the tenant.
COMMUNITY_ID="00000000-0000-4000-8000-00000000c0de"
# Host must match the relay's normalized bind host verbatim (non-default ports
# are kept by normalize_host). Overridable so an isolated relay on an alternate
# port can seed the same channels/members against its own tenant.
COMMUNITY_HOST="${BUZZ_COMMUNITY_HOST:-localhost:3000}"

run_sql "
INSERT INTO communities (id, host)
VALUES ('${COMMUNITY_ID}', '${COMMUNITY_HOST}')
ON CONFLICT (lower(host)) DO NOTHING
;
"

UUID_GENERAL=$(uuid5_hex "buzz.channel.general")
UUID_RANDOM=$(uuid5_hex "buzz.channel.random")
UUID_ENGINEERING=$(uuid5_hex "buzz.channel.engineering")
UUID_AGENTS=$(uuid5_hex "buzz.channel.agents")
UUID_WATERCOOLER=$(uuid5_hex "buzz.channel.watercooler")
UUID_ANNOUNCEMENTS=$(uuid5_hex "buzz.channel.announcements")
UUID_DM_ALICE_TYLER=$(uuid5_hex "buzz.channel.dm.alice-tyler")
UUID_DM_BOB_TYLER=$(uuid5_hex "buzz.channel.dm.bob-tyler")
UUID_DM_BOB_CHARLIE_TYLER=$(uuid5_hex "buzz.channel.dm.bob-charlie-tyler")

run_sql "
INSERT INTO channels
  (community_id, id, name, channel_type, visibility, description, created_by, topic_required)
VALUES
  ('${COMMUNITY_ID}', '${UUID_GENERAL}', 'general', 'stream', 'open', 'General discussion for everyone', decode('${SYSTEM_PUBKEY}','hex'), false),
  ('${COMMUNITY_ID}', '${UUID_RANDOM}', 'random', 'stream', 'open', 'Off-topic, fun stuff', decode('${SYSTEM_PUBKEY}','hex'), false),
  ('${COMMUNITY_ID}', '${UUID_ENGINEERING}', 'engineering', 'stream', 'open', 'Engineering discussions', decode('${SYSTEM_PUBKEY}','hex'), false),
  ('${COMMUNITY_ID}', '${UUID_AGENTS}', 'agents', 'stream', 'open', 'AI agent testing and collaboration', decode('${SYSTEM_PUBKEY}','hex'), false),
  ('${COMMUNITY_ID}', '${UUID_WATERCOOLER}', 'watercooler', 'forum', 'open', 'Casual forum for async discussions', decode('${SYSTEM_PUBKEY}','hex'), true),
  ('${COMMUNITY_ID}', '${UUID_ANNOUNCEMENTS}', 'announcements', 'forum', 'open', 'Company announcements', decode('${SYSTEM_PUBKEY}','hex'), true),
  ('${COMMUNITY_ID}', '${UUID_DM_ALICE_TYLER}', 'alice-tyler', 'dm', 'private', 'DM between alice and tyler', decode('${SYSTEM_PUBKEY}','hex'), false),
  ('${COMMUNITY_ID}', '${UUID_DM_BOB_TYLER}', 'bob-tyler', 'dm', 'private', 'DM between bob and tyler', decode('${SYSTEM_PUBKEY}','hex'), false),
  ('${COMMUNITY_ID}', '${UUID_DM_BOB_CHARLIE_TYLER}', 'bob-charlie-tyler', 'dm', 'private', 'Group DM: bob, charlie, tyler', decode('${SYSTEM_PUBKEY}','hex'), false)
ON CONFLICT DO NOTHING
;
"

run_sql "
INSERT INTO channel_members
  (community_id, channel_id, pubkey, role, invited_by)
VALUES
  ('${COMMUNITY_ID}', '${UUID_GENERAL}', decode('${TYLER_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_GENERAL}', decode('${ALICE_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_GENERAL}', decode('${BOB_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_RANDOM}', decode('${TYLER_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_ENGINEERING}', decode('${TYLER_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_AGENTS}', decode('${TYLER_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_WATERCOOLER}', decode('${TYLER_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_ANNOUNCEMENTS}', decode('${TYLER_PUBKEY}','hex'), 'guest', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_DM_ALICE_TYLER}', decode('${ALICE_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_DM_ALICE_TYLER}', decode('${TYLER_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_DM_BOB_TYLER}', decode('${BOB_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_DM_BOB_TYLER}', decode('${TYLER_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_DM_BOB_CHARLIE_TYLER}', decode('${BOB_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_DM_BOB_CHARLIE_TYLER}', decode('${CHARLIE_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_DM_BOB_CHARLIE_TYLER}', decode('${TYLER_PUBKEY}','hex'), 'member', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_GENERAL}', decode('${AGENT_PUBKEY}','hex'), 'bot', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_RANDOM}', decode('${AGENT_PUBKEY}','hex'), 'bot', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_ENGINEERING}', decode('${AGENT_PUBKEY}','hex'), 'bot', decode('${SYSTEM_PUBKEY}','hex')),
  ('${COMMUNITY_ID}', '${UUID_AGENTS}', decode('${AGENT_PUBKEY}','hex'), 'bot', decode('${SYSTEM_PUBKEY}','hex'))
ON CONFLICT DO NOTHING
;
"

echo "Desktop e2e data ready."
