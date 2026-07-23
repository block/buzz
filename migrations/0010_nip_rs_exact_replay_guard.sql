-- Replace the published 0009 watermark guard without changing its checksum.
-- Exact replay is a durable coordinate-level no-op, independent of whether the
-- physically retained payload still exists.
CREATE OR REPLACE FUNCTION guard_nip_rs_watermark() RETURNS trigger AS $$
DECLARE
    advanced BOOLEAN;
BEGIN
    IF NEW.kind = 30078
       AND NEW.d_tag ~ '^read-state:[0-9a-f]{32}$'
       AND EXISTS (
           SELECT 1
           FROM jsonb_array_elements(NEW.tags) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND jsonb_array_length(tag) = 2
             AND tag->>0 = 't'
             AND tag->>1 = 'read-state'
       ) THEN
        INSERT INTO parameterized_event_watermarks
            (community_id, kind, pubkey, d_tag, created_at, event_id)
        VALUES
            (NEW.community_id, NEW.kind, NEW.pubkey, NEW.d_tag, NEW.created_at, NEW.id)
        ON CONFLICT (community_id, kind, pubkey, d_tag) DO UPDATE SET
            created_at = EXCLUDED.created_at,
            event_id = EXCLUDED.event_id
        WHERE EXCLUDED.created_at > parameterized_event_watermarks.created_at
           OR (EXCLUDED.created_at = parameterized_event_watermarks.created_at
               AND EXCLUDED.event_id < parameterized_event_watermarks.event_id)
        RETURNING TRUE INTO advanced;

        IF NOT COALESCE(advanced, FALSE) THEN
            -- Exact equality is idempotent at the durable coordinate level,
            -- whether or not its payload is still live. Skip it in the trigger
            -- so concurrent physical deletion cannot create a resurrection
            -- window between an existence check and uniqueness enforcement.
            IF EXISTS (
                SELECT 1
                FROM parameterized_event_watermarks
                WHERE community_id = NEW.community_id
                  AND kind = NEW.kind
                  AND pubkey = NEW.pubkey
                  AND d_tag = NEW.d_tag
                  AND created_at = NEW.created_at
                  AND event_id = NEW.id
            ) THEN
                RETURN NULL;
            END IF;

            RAISE EXCEPTION 'stale NIP-RS event rejected by durable watermark'
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
