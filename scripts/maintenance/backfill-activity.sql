-- Backfill what GET /buzz/v1/me/activity reads from rows written before the
-- relay that keeps them (migration 0105). See docs/buzz-v1-activity.md.
--
-- For messages created within the last 30 days it fills:
--   * event_mentions.received_at and root_id, so the mention is in the feed;
--   * personal_read_frontiers rows for the people a thread reply pulls into
--     its thread (its author, the author of the message it answers, and the
--     members it tags), so the thread is in their list.
--
-- Run in psql, outside a transaction, after every relay runs the release
-- that writes these rows:
--   psql -X -v ON_ERROR_STOP=1 -f scripts/maintenance/backfill-activity.sql
-- Then run `buzz-admin warm-activity` for each community.
--
-- Safe to rerun and to stop: it works through each community in windows of at
-- most one hour and 5,000 messages, each committed on its own. A window fills
-- only what is missing and never moves a read position or an engagement time
-- backward. A failed window (a lock wait, for example) stops the script; run
-- it again.
--
-- A thread it adds to a person's list reaches a client that already holds a
-- marker: the person's change counter is advanced as their own writes do.
\set ON_ERROR_STOP on

CREATE TEMP TABLE activity_backfill_bounds ON COMMIT PRESERVE ROWS AS
    SELECT clock_timestamp() - interval '30 days' AS lower, clock_timestamp() AS upper;
CREATE TEMP TABLE activity_backfill_people (
    actor BYTEA NOT NULL,
    channel_id UUID NOT NULL,
    root_id BYTEA NOT NULL,
    engaged_at TIMESTAMPTZ NOT NULL,
    read_at TIMESTAMPTZ,
    read_id BYTEA
) ON COMMIT PRESERVE ROWS;

DO $backfill$
DECLARE
    eligible CONSTANT int[] := ARRAY[9, 40002, 45001, 45003];
    step CONSTANT interval := interval '1 hour';
    step_rows CONSTANT int := 5000;
    tagged_per_reply CONSTANT int := 64;
    cut timestamptz;
    lower timestamptz;
    upper timestamptz;
    communities uuid[];
    community uuid;
    from_at timestamptz;
    to_at timestamptz;
    active boolean;
BEGIN
    SELECT b.lower, b.upper INTO lower, upper FROM activity_backfill_bounds b;
    SELECT COALESCE(array_agg(c.id ORDER BY c.id), '{}') INTO communities FROM communities c;
    COMMIT;
    FOREACH community IN ARRAY communities LOOP
        from_at := lower;
        WHILE from_at < upper LOOP
            to_at := LEAST(from_at + step, upper);
            -- A busy hour is cut where its 5,000th message was created.
            -- Author times are whole seconds, so a cut moves on by at least one.
            SELECT e.created_at INTO cut FROM events e
                WHERE e.community_id = community
                    AND e.created_at >= from_at AND e.created_at < to_at
                ORDER BY e.created_at OFFSET step_rows LIMIT 1;
            IF cut IS NOT NULL THEN
                to_at := LEAST(to_at, GREATEST(cut, from_at + interval '1 second'));
            END IF;
            SET LOCAL lock_timeout = '5s';
            SET LOCAL statement_timeout = '5min';
            -- Community deletion takes this lock exclusively. Read the state
            -- only once it is held.
            PERFORM pg_advisory_xact_lock_shared(
                hashtextextended('buzz-community-deletion:' || community::text, 0));
            SELECT c.deletion_state = 'active' AND c.deleted_at IS NULL INTO active
                FROM communities c WHERE c.id = community;
            IF active IS NOT TRUE THEN
                COMMIT;
                EXIT;
            END IF;

            UPDATE event_mentions m
            SET received_at = e.received_at,
                root_id = (SELECT tm.root_event_id FROM thread_metadata tm
                    WHERE tm.community_id = e.community_id
                        AND tm.event_created_at = e.created_at AND tm.event_id = e.id
                        AND tm.depth >= 1 AND tm.root_event_id <> e.id)
            FROM events e
            WHERE e.community_id = community
                AND e.created_at >= from_at AND e.created_at < to_at
                AND e.kind = ANY(eligible)
                AND m.community_id = e.community_id AND m.event_id = e.id
                AND m.received_at IS NULL;

            -- One row per (person, thread) a reply in the window pulls in.
            -- Only the author has read the reply.
            DELETE FROM activity_backfill_people;
            INSERT INTO activity_backfill_people
            WITH replies AS (
                SELECT tm.channel_id, tm.root_event_id AS root_id, e.id, e.pubkey,
                    e.received_at, tm.parent_event_id, tm.parent_event_created_at
                FROM thread_metadata tm
                JOIN events e ON e.community_id = tm.community_id
                    AND e.created_at = tm.event_created_at AND e.id = tm.event_id
                WHERE tm.community_id = community
                    AND tm.event_created_at >= from_at AND tm.event_created_at < to_at
                    AND tm.depth >= 1 AND octet_length(tm.root_event_id) = 32
                    AND tm.root_event_id <> tm.event_id
                    AND e.kind = ANY(eligible) AND e.deleted_at IS NULL
            ), people AS (
                SELECT r.pubkey AS actor, r.channel_id, r.root_id,
                    r.received_at AS engaged_at, r.received_at AS read_at, r.id AS read_id
                FROM replies r
                UNION ALL
                SELECT p.pubkey, r.channel_id, r.root_id, r.received_at, NULL, NULL
                FROM replies r
                JOIN events p ON p.community_id = community
                    AND p.created_at = r.parent_event_created_at
                    AND p.id = r.parent_event_id AND p.channel_id = r.channel_id
                UNION ALL
                -- At most 64 of the people a reply tags, as the relay takes.
                SELECT decode(m.pubkey_hex, 'hex'), r.channel_id, r.root_id, r.received_at,
                    NULL, NULL
                FROM replies r
                CROSS JOIN LATERAL (SELECT m.pubkey_hex FROM event_mentions m
                    WHERE m.community_id = community AND m.event_id = r.id
                        AND m.pubkey_hex ~ '^[0-9a-f]{64}$'
                        AND m.pubkey_hex <> encode(r.pubkey, 'hex')
                    ORDER BY m.pubkey_hex LIMIT tagged_per_reply) m
            )
            SELECT p.actor, p.channel_id, p.root_id, max(p.engaged_at), max(p.read_at),
                (array_agg(p.read_id ORDER BY p.read_at DESC NULLS LAST))[1]
            FROM people p
            WHERE octet_length(p.actor) = 32
            GROUP BY p.actor, p.channel_id, p.root_id
            -- Anyone but an author must still be a member of the channel.
            HAVING max(p.read_at) IS NOT NULL OR EXISTS (SELECT 1 FROM channel_members cm
                WHERE cm.community_id = community AND cm.channel_id = p.channel_id
                    AND cm.pubkey = p.actor AND cm.removed_at IS NULL);

            -- Keep only what would change a row, so a rerun writes nothing.
            DELETE FROM activity_backfill_people p USING personal_read_frontiers f
            WHERE f.community_id = community AND f.actor = p.actor
                AND f.channel_id = p.channel_id AND f.root_id = p.root_id
                AND f.engaged_at >= p.engaged_at
                AND (p.read_at IS NULL OR f.through_timestamp >= p.read_at);

            -- New accounts commit on their own, so the relay never waits on
            -- one of these rows for the length of a window.
            INSERT INTO personal_read_accounts (community_id, actor)
            SELECT DISTINCT community, p.actor FROM activity_backfill_people p ORDER BY p.actor
            ON CONFLICT (community_id, actor) DO NOTHING;
            COMMIT;

            SET LOCAL lock_timeout = '5s';
            SET LOCAL statement_timeout = '5min';
            PERFORM pg_advisory_xact_lock_shared(
                hashtextextended('buzz-community-deletion:' || community::text, 0));
            SELECT c.deletion_state = 'active' AND c.deleted_at IS NULL INTO active
                FROM communities c WHERE c.id = community;
            IF active IS NOT TRUE THEN
                COMMIT;
                EXIT;
            END IF;

            -- Account, then frontier: the order the relay takes them. Each
            -- person's counter advances once and stamps their rows, so a
            -- `since` read reports the threads.
            UPDATE personal_read_accounts a SET seq = a.seq + 1
            FROM (SELECT DISTINCT p.actor FROM activity_backfill_people p ORDER BY p.actor) p
            WHERE a.community_id = community AND a.actor = p.actor;

            INSERT INTO personal_read_frontiers AS f (community_id, actor, channel_id, root_id,
                through_timestamp, through_message_id, engaged_at, seq)
            SELECT community, p.actor, p.channel_id, p.root_id, p.read_at, p.read_id,
                p.engaged_at, a.seq
            FROM activity_backfill_people p
            JOIN personal_read_accounts a ON a.community_id = community AND a.actor = p.actor
            -- The channel may have been deleted since the first transaction.
            WHERE EXISTS (SELECT 1 FROM channels c
                WHERE c.community_id = community AND c.id = p.channel_id)
            ORDER BY p.channel_id, p.actor, p.root_id
            ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE SET
                through_message_id = CASE WHEN EXCLUDED.through_timestamp
                        > COALESCE(f.through_timestamp, '-infinity')
                    THEN EXCLUDED.through_message_id ELSE f.through_message_id END,
                through_timestamp = GREATEST(f.through_timestamp, EXCLUDED.through_timestamp),
                engaged_at = GREATEST(f.engaged_at, EXCLUDED.engaged_at),
                seq = EXCLUDED.seq;

            COMMIT;
            from_at := to_at;
        END LOOP;
    END LOOP;
END
$backfill$;

-- Validation. Both counts are 0 when the backfill is complete: no message in
-- the range still has a mention without an arrival time, and no reply author
-- is missing from their thread.
SELECT
    (SELECT count(*) FROM communities c
        JOIN events e ON e.community_id = c.id
            AND e.created_at >= b.lower AND e.created_at < b.upper
            AND e.kind = ANY(ARRAY[9, 40002, 45001, 45003])
        JOIN event_mentions m ON m.community_id = e.community_id AND m.event_id = e.id
        WHERE c.deletion_state = 'active' AND c.deleted_at IS NULL
            AND m.received_at IS NULL) AS mentions_without_arrival,
    (SELECT count(*) FROM communities c
        JOIN thread_metadata tm ON tm.community_id = c.id
            AND tm.event_created_at >= b.lower AND tm.event_created_at < b.upper
            AND tm.depth >= 1 AND octet_length(tm.root_event_id) = 32
            AND tm.root_event_id <> tm.event_id
        JOIN events e ON e.community_id = tm.community_id
            AND e.created_at = tm.event_created_at AND e.id = tm.event_id
            AND e.kind = ANY(ARRAY[9, 40002, 45001, 45003]) AND e.deleted_at IS NULL
        WHERE c.deletion_state = 'active' AND c.deleted_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM personal_read_frontiers f
                WHERE f.community_id = tm.community_id AND f.actor = e.pubkey
                    AND f.channel_id = tm.channel_id
                    AND f.root_id = tm.root_event_id)) AS reply_authors_without_thread
FROM activity_backfill_bounds b;
