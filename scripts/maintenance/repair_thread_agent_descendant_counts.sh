#!/usr/bin/env bash
# OUT-OF-BAND MAINTENANCE: recounts thread_agent_reply_counts (migration 0089)
# for thread roots written before the relay counted `audience=agents` replies.
# Until it runs, those threads report 0 agent replies, so clients show every
# reply, as they did before the field existed.
#
# Run it only after every serving relay counts agent-audience replies on
# insert and delete (the release that ships migration 0089) and older relays
# have drained. An older relay keeps adding and deleting replies without
# updating the table, so counts it touches would drift again.
#
# Safe to rerun: each root's count is recomputed from its live replies, never
# incremented, and a root already correct is not written. Each batch of up to
# BATCH_SIZE roots in one community is its own psql transaction with its own
# lock and statement timeouts, so the run's length is unbounded but no
# statement is. A batch takes the community's shared deletion lock, then
# repairs only if that community is still active, as
# DeletionStore::guard_transaction does. It then locks its roots, so a reply
# insert or delete under them (which updates its root's thread_metadata row in
# the same transaction) waits until the batch commits, and the recount, a
# later statement with a fresh snapshot, sees every reply committed before it.
# A batch that fails, for example on a lock timeout, is retried with backoff;
# after MAX_ATTEMPTS the script stops, and a rerun starts from the beginning.
#
# Usage:
#   DATABASE_URL=postgres://... \
#     scripts/maintenance/repair_thread_agent_descendant_counts.sh
#
# It prints one line per community it repaired and, last, the validation
# result. The expected result is 0 on a database with no concurrent
# agent-audience writes; it covers the same roots the repair does.
set -euo pipefail

: "${DATABASE_URL:?set DATABASE_URL}"
batch_size=${BATCH_SIZE:-100}
max_attempts=${MAX_ATTEMPTS:-5}
[[ $batch_size =~ ^[1-9][0-9]*$ ]] || {
    echo "BATCH_SIZE must be a positive integer" >&2
    exit 2
}

psql_base=(psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 -At -q)

# Shared by the repair and the validation: the stored events under a root
# whose tags hold a tag starting ["audience", "agents"], as the relay's
# agent_audience_predicate! tests them.
agent_replies_sql='
    SELECT count(*)::INT
      FROM thread_metadata reply
      JOIN events e ON e.community_id = reply.community_id
                   AND e.created_at = reply.event_created_at
                   AND e.id = reply.event_id
     WHERE reply.community_id = root.community_id
       AND reply.root_event_id = root.event_id
       AND e.deleted_at IS NULL
       AND jsonb_path_exists(e.tags,
             '\''strict $[*] ? (@[0] == "audience" && @[1] == "agents")'\'',
             '\''{}'\'', true)'

# Roots the repair covers: any with replies, and any with no replies left
# but a stored count, which an older relay's delete can leave behind.
needs_count_sql='(root.descendant_count > 0 OR EXISTS (
            SELECT 1 FROM thread_agent_reply_counts stored
             WHERE stored.community_id = root.community_id
               AND stored.root_event_id = root.event_id
               AND stored.agent_descendant_count <> 0))'

# Prints `more|last_created_at|last_event_id_hex|changed`.
run_batch() {
    local community=$1 after_created_at=$2 after_event_id=$3
    "${psql_base[@]}" \
        -v community="$community" \
        -v after_created_at="$after_created_at" \
        -v after_event_id="$after_event_id" \
        -v batch_size="$batch_size" <<SQL
SET TimeZone = 'UTC';
BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
SELECT pg_advisory_xact_lock_shared(community_deletion_lock_key(:'community'::UUID)) \gset lock_
SELECT count(*) > 0 AS more,
       coalesce(max(event_created_at)::TEXT, '') AS last_created_at,
       coalesce((array_agg(encode(event_id, 'hex') ORDER BY event_created_at DESC, event_id DESC))[1], '') AS last_event_id,
       coalesce(array_agg(event_created_at ORDER BY event_created_at, event_id)::TEXT, '{}') AS created_ats,
       coalesce(array_agg(encode(event_id, 'hex') ORDER BY event_created_at, event_id)::TEXT, '{}') AS event_ids
  FROM (
        SELECT root.event_created_at, root.event_id
          FROM thread_metadata root
         WHERE root.community_id = :'community'::UUID
           AND root.depth = 0
           AND ${needs_count_sql}
           AND (root.event_created_at, root.event_id)
               > (:'after_created_at'::TIMESTAMPTZ, decode(:'after_event_id', 'hex'))
           AND EXISTS (
                SELECT 1 FROM communities
                 WHERE id = :'community'::UUID
                   AND deletion_state = 'active'
                   AND deleted_at IS NULL)
         ORDER BY root.event_created_at, root.event_id
         LIMIT :batch_size
           FOR UPDATE
       ) batch \gset
\set changed 0
\if :more
WITH recount AS (
        SELECT root.community_id, root.event_id, (${agent_replies_sql}) AS agents
          FROM unnest(:'created_ats'::TIMESTAMPTZ[], :'event_ids'::TEXT[])
               AS target(created_at, event_id_hex)
          JOIN thread_metadata root
            ON root.community_id = :'community'::UUID
           AND root.event_created_at = target.created_at
           AND root.event_id = decode(target.event_id_hex, 'hex')
     ), upserted AS (
        INSERT INTO thread_agent_reply_counts
               (community_id, root_event_id, agent_descendant_count)
        SELECT recount.community_id, recount.event_id, recount.agents
          FROM recount
          LEFT JOIN thread_agent_reply_counts current
            ON current.community_id = recount.community_id
           AND current.root_event_id = recount.event_id
         WHERE recount.agents <> coalesce(current.agent_descendant_count, 0)
        ON CONFLICT (community_id, root_event_id) DO UPDATE
           SET agent_descendant_count = EXCLUDED.agent_descendant_count
        RETURNING 1
     )
SELECT count(*) AS changed FROM upserted \gset
\endif
COMMIT;
\echo :more|:last_created_at|:last_event_id|:changed
SQL
}

# Read before the loop: a failed query in a process substitution would not
# stop the script under set -e.
communities=$("${psql_base[@]}" -c "SELECT id FROM communities ORDER BY id")
total=0
while IFS= read -r community; do
    [[ -n $community ]] || continue
    after_created_at=-infinity
    after_event_id=
    repaired=0
    while :; do
        attempt=1
        until result=$(run_batch "$community" "$after_created_at" "$after_event_id"); do
            if ((attempt >= max_attempts)); then
                echo "community $community: batch after ($after_created_at, $after_event_id) failed $attempt times; stopping" >&2
                exit 1
            fi
            sleep "$attempt"
            attempt=$((attempt + 1))
        done
        IFS='|' read -r more last_created_at last_event_id changed <<<"$result"
        repaired=$((repaired + changed))
        [[ $more == t ]] || break
        after_created_at=$last_created_at
        after_event_id=$last_event_id
    done
    if ((repaired > 0)); then
        echo "community $community: repaired $repaired thread roots"
    fi
    total=$((total + repaired))
done <<<"$communities"
echo "repaired $total thread roots"

# One check over the active communities' roots, run once at the end. The
# timeout stops a slow scan from hanging the end of a long run; a timeout
# fails the script after the repair has committed.
mismatched=$("${psql_base[@]}" -c "
SET statement_timeout = '5min';
SELECT count(*)
  FROM thread_metadata root
  JOIN communities c ON c.id = root.community_id
                    AND c.deletion_state = 'active'
                    AND c.deleted_at IS NULL
  LEFT JOIN thread_agent_reply_counts current
    ON current.community_id = root.community_id
   AND current.root_event_id = root.event_id
 WHERE root.depth = 0
   AND ${needs_count_sql}
   AND coalesce(current.agent_descendant_count, 0) <> (${agent_replies_sql})")
echo "validation: $mismatched thread roots still differ (expected 0)"
