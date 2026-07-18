-- no-transaction
-- Partial index for per-user logical-storage queries against the audit log.
-- object_id trails the key so the per-user dedup subquery (community_id,
-- actor_pubkey, object_id) drives off an Index Scan on this index instead of
-- a sequential scan; Postgres still adds an Incremental Sort + Unique to
-- pick the max-size row per (community_id, actor_pubkey, object_id), since
-- the DISTINCT ON tiebreaker column (blob_bytes) isn't part of the index.
-- CREATE INDEX CONCURRENTLY requires running outside a transaction (SQLx
-- marker above).
--
-- Retry safety has two layers:
-- 1. Startup repair guard (repair_invalid_media_index in migration.rs):
--    drops any indisvalid=false leftover from a failed concurrent build
--    before the migrator runs, so a fresh valid build can proceed.
-- 2. IF NOT EXISTS: if a prior build succeeded but SQLx died before
--    recording version 20 (bookkeeping race), the valid index already
--    exists — PG skips creation and SQLx records version 20 normally.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_audit_log_media_uploads
    ON audit_log (community_id, actor_pubkey, object_id)
    WHERE action = 'media_uploaded';
