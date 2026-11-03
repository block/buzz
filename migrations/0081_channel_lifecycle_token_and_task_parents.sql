-- Guarded task archive and restore (NEXUS-4111).
--
-- `channels.lifecycle_token` is nullable with no default, so adding it changes
-- only the catalog: no table rewrite and no backfill. NULL means the row has
-- not changed lifecycle state since this migration. The relay writes a fresh
-- random value, generated in Rust, on every create, archive, restore, delete,
-- reap and lifecycle fence. Clients compare tokens for equality only.
--
-- `artifact_parents` projects the hierarchy of relay-validated `nexus.task`
-- records: one row per live task head, written in the record's
-- compare-and-swap transaction. `parent_channel_id` is NULL for a root. Old
-- relays never read or write it.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE channels ADD COLUMN lifecycle_token UUID;

CREATE TABLE artifact_parents (
    community_id UUID NOT NULL,
    child_channel_id UUID NOT NULL,
    parent_channel_id UUID,
    artifact_id UUID NOT NULL,
    PRIMARY KEY (community_id, child_channel_id)
);
CREATE INDEX artifact_parents_parent ON artifact_parents (community_id, parent_channel_id)
    WHERE parent_channel_id IS NOT NULL;
