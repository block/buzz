-- Refresh ephemeral-channel expiry in the transaction that makes a
-- channel-scoped event durable. Deferring to COMMIT closes the stale-prefetch
-- race: the UPDATE sees a TTL transition committed while ingest was in flight,
-- or waits on its row lock and rechecks after it commits, without restoring a
-- separate hot-path transaction.
CREATE FUNCTION refresh_channel_ttl_after_event_insert() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    -- Kind 9007 creates the channel and initializes its deadline itself.
    IF NEW.channel_id IS NOT NULL AND NEW.kind <> 9007 THEN
        BEGIN
            -- Lock by identity before testing ttl_seconds. If a concurrent TTL
            -- transition is uncommitted, this waits and follows its updated row
            -- version instead of treating the old permanent version as final.
            PERFORM 1 FROM channels
            WHERE community_id = NEW.community_id AND id = NEW.channel_id
            FOR UPDATE;

            UPDATE channels
            SET ttl_deadline = clock_timestamp() + make_interval(secs => ttl_seconds)
            WHERE community_id = NEW.community_id
              AND id = NEW.channel_id
              AND ttl_seconds IS NOT NULL
              AND archived_at IS NULL
              AND deleted_at IS NULL;
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

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
AFTER INSERT ON events
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();
