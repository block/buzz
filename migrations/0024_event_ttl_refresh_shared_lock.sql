-- T1a repair: the 0022 trigger takes FOR UPDATE on the channel row before
-- testing ttl_seconds, so every durable message in a PERMANENT channel
-- serializes on that tuple at commit time (deferred trigger) — one hot
-- channel means fully serialized commits, each holding the lock across its
-- WAL flush. Observed live at 200 QPS: commit latency 0.07ms -> ~15ms,
-- non-CPU DB load ~9/vCPU with CPU under 45%.
--
-- Repair keeps 0022's stale-prefetch proof but moves the synchronization to
-- a per-channel advisory lock, shared on the hot path:
--   * Event insert: shared channel-key lock -> read ttl_seconds. NULL returns
--     with no tuple lock and no update; shared locks admit each other, so
--     permanent-channel commits proceed concurrently.
--   * Permanent->ephemeral (or TTL-change) transition (update_channel in
--     crates/buzz-db/src/channel.rs) takes the same key EXCLUSIVE before its
--     UPDATE. Either the transition commits first and the event's read sees
--     the TTL (and refreshes), or the event commits first and the
--     transition's own deadline reset is later than anything the event would
--     have written. No stale-NULL hole in either order.
--   * Ephemeral channels still run the conditional UPDATE; their row updates
--     serialize per channel, but only ephemeral channels pay that.
-- Lock key domain 'buzz_channel_ttl:' is distinct from 'buzz_push_gate:'
-- (migration 0023) and the audit/lease lock families. Lock order note: the
-- deferred trigger acquires this key at COMMIT, after any push-gate shared
-- lock taken during insert; no path acquires both domains exclusively.
CREATE OR REPLACE FUNCTION refresh_channel_ttl_after_event_insert() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    channel_ttl INTEGER;
BEGIN
    -- Kind 9007 creates the channel and initializes its deadline itself.
    IF NEW.channel_id IS NOT NULL AND NEW.kind <> 9007 THEN
        BEGIN
            PERFORM pg_advisory_xact_lock_shared(hashtextextended(
                'buzz_channel_ttl:' || NEW.community_id::text || ':' || NEW.channel_id::text, 0));

            SELECT ttl_seconds INTO channel_ttl
            FROM channels
            WHERE community_id = NEW.community_id AND id = NEW.channel_id;

            IF channel_ttl IS NOT NULL THEN
                UPDATE channels
                SET ttl_deadline = clock_timestamp() + make_interval(secs => ttl_seconds)
                WHERE community_id = NEW.community_id
                  AND id = NEW.channel_id
                  AND ttl_seconds IS NOT NULL
                  AND archived_at IS NULL
                  AND deleted_at IS NULL;
            END IF;
        EXCEPTION WHEN OTHERS THEN
            -- Preserve the existing best-effort contract: a TTL refresh failure
            -- must not reject an otherwise valid durable event.
            RAISE WARNING 'channel TTL refresh failed for community %, channel %: %',
                NEW.community_id, NEW.channel_id, SQLERRM;
        END;
    END IF;
    RETURN NULL;
END
$$;
