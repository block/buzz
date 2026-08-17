-- Match NIP-RS's exact tag cardinality in mixed-version database guards.
-- Published migrations 0007-0010 remain checksum-frozen.
-- Remove polluted watermarks only when their exact source payload still exists
-- and proves the event was nonconforming. Missing source payloads are left
-- untouched: they may be legitimate NIP-09-deleted read state and have no
-- remaining provenance that permits safe automatic classification.
DELETE FROM parameterized_event_watermarks watermark
USING events source
WHERE source.community_id = watermark.community_id
  AND source.kind = watermark.kind
  AND source.pubkey = watermark.pubkey
  AND source.d_tag = watermark.d_tag
  AND source.created_at = watermark.created_at
  AND source.id = watermark.event_id
  AND source.kind = 30078
  AND NOT (
      source.d_tag ~ '^read-state:[0-9a-f]{32}$'
      AND (
          SELECT count(*)
          FROM jsonb_array_elements(CASE WHEN jsonb_typeof(source.tags) = 'array' THEN source.tags ELSE '[]'::jsonb END) tag
          WHERE jsonb_typeof(tag) = 'array'
            AND tag->0 = '"d"'::jsonb
      ) = 1
      AND EXISTS (
          SELECT 1
          FROM jsonb_array_elements(CASE WHEN jsonb_typeof(source.tags) = 'array' THEN source.tags ELSE '[]'::jsonb END) tag
          WHERE jsonb_typeof(tag) = 'array'
            AND jsonb_array_length(tag) >= 2
            AND jsonb_typeof(tag->1) = 'string'
            AND tag->>0 = 'd'
            AND tag->>1 = source.d_tag
      )
      AND (
          SELECT count(*)
          FROM jsonb_array_elements(CASE WHEN jsonb_typeof(source.tags) = 'array' THEN source.tags ELSE '[]'::jsonb END) tag
          WHERE tag = '["t", "read-state"]'::jsonb
      ) = 1
  );

-- A relay binary from before this migration can classify an incoming event by
-- broad EXISTS predicates and hard-delete the current coordinate before its
-- corrected INSERT guard runs. Fail the whole old-writer transaction rather
-- than silently skipping the DELETE (which would permit two live rows and
-- strip the retained row's mentions). Corrected paths opt in transaction-locally.
CREATE FUNCTION guard_nip_rs_hard_delete() RETURNS trigger AS $$
BEGIN
    IF current_setting('buzz.nip_rs_hard_delete', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'NIP-RS hard delete requires corrected writer opt-in'
            USING ERRCODE = 'check_violation';
    END IF;

    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_events_guard_nip_rs_hard_delete
    BEFORE DELETE ON events
    FOR EACH ROW
    WHEN (OLD.kind = 30078 AND OLD.d_tag ~ '^read-state:[0-9a-f]{32}$')
    EXECUTE FUNCTION guard_nip_rs_hard_delete();

CREATE OR REPLACE FUNCTION guard_nip_rs_watermark() RETURNS trigger AS $$
DECLARE
    advanced BOOLEAN;
BEGIN
    IF NEW.kind = 30078
       AND NEW.d_tag ~ '^read-state:[0-9a-f]{32}$'
       AND (
           SELECT count(*)
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND tag->0 = '"d"'::jsonb
       ) = 1
       AND EXISTS (
           SELECT 1
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND jsonb_array_length(tag) >= 2
             AND jsonb_typeof(tag->1) = 'string'
             AND tag->>0 = 'd'
             AND tag->>1 = NEW.d_tag
       )
       AND (
           SELECT count(*)
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE tag = '["t", "read-state"]'::jsonb
       ) = 1 THEN
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

CREATE OR REPLACE FUNCTION purge_soft_deleted_nip_rs() RETURNS trigger AS $$
BEGIN
    IF OLD.deleted_at IS NULL
       AND NEW.deleted_at IS NOT NULL
       AND NEW.kind = 30078
       AND NEW.d_tag ~ '^read-state:[0-9a-f]{32}$'
       AND (
           SELECT count(*)
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND tag->0 = '"d"'::jsonb
       ) = 1
       AND EXISTS (
           SELECT 1
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND jsonb_array_length(tag) >= 2
             AND jsonb_typeof(tag->1) = 'string'
             AND tag->>0 = 'd'
             AND tag->>1 = NEW.d_tag
       )
       AND (
           SELECT count(*)
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE tag = '["t", "read-state"]'::jsonb
       ) = 1 THEN
        PERFORM set_config('buzz.nip_rs_hard_delete', 'on', true);

        DELETE FROM events
        WHERE community_id = NEW.community_id
          AND created_at = NEW.created_at
          AND id = NEW.id;

        DELETE FROM event_mentions
        WHERE community_id = NEW.community_id AND event_id = NEW.id;
    END IF;

    RETURN NULL;
END;
$$ LANGUAGE plpgsql;
