-- ── Storage usage history ───────────────────────────────────────────────────
-- Append-only per-community usage per completed accounting run, served as the
-- operator storage-usage feed. usage_community_id is provenance only (no FK);
-- community deletion severs it.

CREATE TABLE storage_usage_history (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    measured_at TIMESTAMPTZ NOT NULL,
    usage_community_id UUID,
    storage_bytes BIGINT NOT NULL CHECK (storage_bytes >= 0),
    storage_objects BIGINT NOT NULL CHECK (storage_objects >= 0)
);

CREATE INDEX idx_storage_usage_history_measured_at
    ON storage_usage_history (measured_at);

CREATE INDEX idx_storage_usage_history_community
    ON storage_usage_history (usage_community_id);
