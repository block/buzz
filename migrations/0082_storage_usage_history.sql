-- Append-only per-community storage usage, one row per community per
-- completed accounting run, written by `buzz-admin storage-snapshot` in the
-- same transaction that replaces storage_accounting_snapshots. The relay
-- serves it as a cursor feed (GET /operator/storage-usage) so an operator
-- can page every run it has not yet read, instead of the relay pushing the
-- latest snapshot and dropping any run replaced before delivery.
--
-- usage_community_id is provenance, not ownership: the worker derives
-- community ids from S3 sidecar keys that may name communities this database
-- no longer has, so there is no foreign key and the column is deliberately
-- not named community_id (docs/database.md, provenance-only checklist).
-- Community deletion severs it to NULL; the feed skips severed rows. Each
-- run prunes rows older than the worker's retention window.
SET LOCAL lock_timeout = '5s';

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

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('storage_usage_history', 'deployment-global per-community storage usage feed; usage_community_id is provenance only');
