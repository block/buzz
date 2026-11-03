-- Hierarchy projection of relay-validated `nexus.task` records: one row per
-- live task head, written in the record's compare-and-swap transaction.
-- `parent_channel_id` is NULL for a root.
CREATE TABLE artifact_parents (
    community_id UUID NOT NULL,
    child_channel_id UUID NOT NULL,
    parent_channel_id UUID,
    artifact_id UUID NOT NULL,
    PRIMARY KEY (community_id, child_channel_id)
);
CREATE INDEX artifact_parents_parent ON artifact_parents (community_id, parent_channel_id)
    WHERE parent_channel_id IS NOT NULL;
