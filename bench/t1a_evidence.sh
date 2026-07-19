#!/usr/bin/env bash
# T1a correctness evidence (runnable from this lane tip).
#
# Proves, against a live relay built from this tree:
#   1. permanent channel: kind:9 ingest emits no separate top-level TTL UPDATE
#      transaction (the deferred trigger's conditional statement stays inside the
#      event transaction and affects zero rows);
#   2. ephemeral channel: the TTL bump is still observed (ttl_deadline strictly advances
#      across a message);
#   3. TTL-set-during-ingest race: messages committed after concurrent TTL activation
#      extend the deadline beyond the activation update's own deadline.
#
# Usage: bench/t1a_evidence.sh <pg_container> <db_url> <relay_ws_url> <community_uuid>
# Requires: relay running FROM THIS TREE against <db_url>; psql via docker exec;
#           BENCH_PRIVATE_KEY env (member secret key hex); wamp_bench built --release.
set -euo pipefail

PG="$1"; DBURL="$2"; RELAY="$3"; COMMUNITY="$4"
PSQL=(docker exec "$PG" psql "$DBURL" -tA)
sql() { "${PSQL[@]}" -c "$1"; }
BIN="./target/release/wamp_bench"
# BENCH_PUB = x-only pubkey hex for BENCH_PRIVATE_KEY (both required).
PUB="${BENCH_PUB:?set BENCH_PUB (x-only pubkey hex matching BENCH_PRIVATE_KEY)}"

mkchan() { # $1 name, $2 ttl_seconds or NULL -> echoes uuid
  local id; id=$(python3 -c "import uuid;print(uuid.uuid4())")
  sql "insert into channels (id, community_id, name, channel_type, visibility, created_by, ttl_seconds, ttl_deadline)
       values ('$id','$COMMUNITY','$1','stream','private',decode('$PUB','hex'),$2,
               case when $2::int is null then null else now() + ($2::int || ' seconds')::interval end)" >/dev/null
  sql "insert into channel_members (community_id, channel_id, pubkey, role)
       values ('$COMMUNITY','$id',decode('$PUB','hex'),'owner')" >/dev/null
  echo "$id"
}

FAIL=0

echo "== 1. permanent channel: zero separate top-level TTL UPDATE statements =="
PERM=$(mkchan t1a-perm NULL)
sql "select pg_stat_statements_reset()" >/dev/null
env -u BUZZ_AUTH_TAG BUZZ_RELAY_URL="$RELAY" "$BIN" "$PERM" 20 10 2 /tmp/t1a-perm.lat >/tmp/t1a-perm.json
ACC=$(python3 -c "import json;print(json.load(open('/tmp/t1a-perm.json'))['accepted'])")
TTL_CALLS=$(sql "select coalesce(sum(calls),0) from pg_stat_statements where query ilike '%UPDATE channels SET ttl_deadline%'")
echo "accepted=$ACC ttl_update_calls=$TTL_CALLS"
[[ "$ACC" -gt 0 && "$TTL_CALLS" == "0" ]] || { echo "FAIL: expected >0 accepted and 0 TTL updates"; FAIL=1; }

echo "== 2. ephemeral channel: bump still observed =="
EPH=$(mkchan t1a-eph 3600)
D0=$(sql "select extract(epoch from ttl_deadline) from channels where id='$EPH'")
sleep 2
env -u BUZZ_AUTH_TAG BUZZ_RELAY_URL="$RELAY" "$BIN" "$EPH" 5 3 1 /tmp/t1a-eph.lat >/tmp/t1a-eph.json
D1=$(sql "select extract(epoch from ttl_deadline) from channels where id='$EPH'")
echo "deadline before=$D0 after=$D1"
python3 -c "import sys; sys.exit(0 if float('$D1') > float('$D0') else 1)" \
  || { echo "FAIL: ephemeral ttl_deadline did not advance"; FAIL=1; }

echo "== 3. TTL-set-during-ingest race =="
RACE=$(mkchan t1a-race NULL)
env -u BUZZ_AUTH_TAG BUZZ_RELAY_URL="$RELAY" "$BIN" "$RACE" 50 6 4 /tmp/t1a-race.lat >/tmp/t1a-race.json &
BPID=$!
sleep 2
# update_channel-equivalent: set TTL and reset deadline in one statement, mid-burst.
ACTIVATION_DEADLINE=$(sql "update channels set ttl_seconds=600, ttl_deadline=clock_timestamp() + interval '600 seconds', updated_at=now() where id='$RACE' and deleted_at is null returning extract(epoch from ttl_deadline)")
wait "$BPID"
ROW=$(sql "select ttl_seconds, extract(epoch from ttl_deadline) from channels where id='$RACE'")
echo "activation_deadline=$ACTIVATION_DEADLINE post-race=$ROW"
FINAL_DEADLINE="${ROW#*|}"
python3 -c "import sys; sys.exit(0 if float('$FINAL_DEADLINE') > float('$ACTIVATION_DEADLINE') else 1)" \
  || { echo "FAIL: later message did not extend TTL beyond activation deadline"; FAIL=1; }
# and subsequent messages now bump it (channel is ephemeral now)
D0=$(sql "select extract(epoch from ttl_deadline) from channels where id='$RACE'")
sleep 2
env -u BUZZ_AUTH_TAG BUZZ_RELAY_URL="$RELAY" "$BIN" "$RACE" 5 3 1 /tmp/t1a-race2.lat >/tmp/t1a-race2.json
D1=$(sql "select extract(epoch from ttl_deadline) from channels where id='$RACE'")
python3 -c "import sys; sys.exit(0 if float('$D1') > float('$D0') else 1)" \
  || { echo "FAIL: post-race ephemeral bump not observed"; FAIL=1; }

[[ "$FAIL" == 0 ]] && echo "T1A EVIDENCE: ALL PASS" || { echo "T1A EVIDENCE: FAILURES"; exit 1; }
