-- T1b push gate (migrations 0023 and 0059). Enqueue only when the
-- community has an active, endpoint-enabled, unexpired lease; the shared
-- advisory lock pairs with the exclusive lock taken by lease activations
-- (crates/buzz-db/src/push.rs) to close the lost-wake race.
CREATE FUNCTION enqueue_push_match_job() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    -- Legacy connections without the setting preserve their existing behavior.
    -- Updated writer pools always set it, including on reconnect. Check before
    -- taking push locks or touching leases/queues so disabled chat stays healthy.
    IF COALESCE(current_setting('buzz.push_async_enqueue', true), '') = 'on' THEN
        RETURN NEW;
    END IF;
    IF NOT COALESCE(NULLIF(current_setting('buzz.push_enabled', true), '')::boolean, true) THEN
        RETURN NEW;
    END IF;
    -- Keep this allowlist identical to the relay's validated NIP-PL descriptor.
    -- This compatibility path covers legacy writers until they are replaced.
    IF NEW.kind IN (9, 40002, 45001, 45003) THEN
        PERFORM pg_advisory_xact_lock_shared(
            hashtextextended('buzz_push_gate:' || NEW.community_id::text, 0));
        IF EXISTS (
            SELECT 1 FROM push_leases
            WHERE community_id = NEW.community_id
              AND active
              AND endpoint_enabled
              AND expires_at > EXTRACT(EPOCH FROM now())::bigint
        ) THEN
            INSERT INTO push_match_queue (community_id, event_id)
            VALUES (NEW.community_id, NEW.id)
            ON CONFLICT DO NOTHING;
        END IF;
    END IF;
    RETURN NEW;
END
$$;
