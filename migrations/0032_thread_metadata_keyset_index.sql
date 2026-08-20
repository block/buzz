-- ── Thread reply keyset pagination index ─────────────────────────────────────
-- Thread subtree reads filter by tenant and root, then page in ascending
-- (event_created_at, event_id) order. Include the keyset columns so PostgreSQL
-- can satisfy both the filter and ordering from one index scan.
--
-- Lock note: built without CONCURRENTLY because sqlx runs each migration inside
-- a transaction and CREATE INDEX CONCURRENTLY cannot run in one. This takes a
-- SHARE lock on `thread_metadata` (blocking writes, not reads) for the duration
-- of the build. `thread_metadata` is on the event insertion path and may be
-- large on brownfield databases, so operators should pre-build it by hand before
-- deploying this migration:
--
--   CREATE INDEX CONCURRENTLY idx_thread_metadata_root_keyset
--       ON thread_metadata (community_id, root_event_id, event_created_at, event_id);
--
-- IF NOT EXISTS then makes this migration a no-op on that database.
--
-- Additive migration: previously applied files must not change checksum.

CREATE INDEX IF NOT EXISTS idx_thread_metadata_root_keyset
    ON thread_metadata (community_id, root_event_id, event_created_at, event_id);
