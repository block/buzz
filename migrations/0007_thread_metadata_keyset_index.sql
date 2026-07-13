-- ── Thread reply keyset pagination index ─────────────────────────────────────
-- Thread subtree reads filter by tenant and root, then page in ascending
-- (event_created_at, event_id) order. Include the keyset columns so PostgreSQL
-- can satisfy both the filter and ordering from one index scan.
--
-- Additive migration: previously applied files must not change checksum.

CREATE INDEX idx_thread_metadata_root_keyset
    ON thread_metadata (community_id, root_event_id, event_created_at, event_id);
