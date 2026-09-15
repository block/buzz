-- Index references in existing events as well as new publications. The optional
-- host filter keeps a foreign relay URL from granting access to a local blob.
CREATE FUNCTION buzz_media_hashes(body TEXT, tags JSONB, media_host TEXT DEFAULT NULL)
RETURNS TEXT[] LANGUAGE SQL IMMUTABLE PARALLEL SAFE AS $$
    SELECT COALESCE(array_agg(DISTINCT lower(parts[2])), ARRAY[]::TEXT[])
    FROM regexp_matches(
        replace(body || ' ' || tags::TEXT, E'\\/', '/'),
        '(?:https?://([^/[:space:]"<>]+))?/media/([0-9a-f]{64})(?=$|[^0-9a-f])',
        'g'
    ) AS parts
    WHERE media_host IS NULL OR parts[1] IS NULL OR lower(parts[1]) = lower(media_host)
$$;

CREATE INDEX idx_events_media_hashes ON events USING GIN (buzz_media_hashes(content, tags));

-- A successful upload proves possession of the bytes, not merely knowledge of
-- their hash. Keep this independently of optional moderation/audit recording.
CREATE TABLE media_uploaders (
    community_id UUID NOT NULL REFERENCES communities(id),
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    pubkey BYTEA NOT NULL CHECK (length(pubkey) = 32),
    PRIMARY KEY (community_id, sha256, pubkey)
);

SELECT attach_community_write_fence('media_uploaders'::regclass);
