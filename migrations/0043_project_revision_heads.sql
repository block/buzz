-- Effective head for actor-signed collaborative Project revisions.
--
-- The signed operation events remain the portable audit trail in `events`.
-- This row is the relay's transactional CAS/materialized state, keyed by the
-- stable owner-authored kind:30621 coordinate.
CREATE TABLE project_revision_heads (
    community_id UUID NOT NULL,
    project_owner BYTEA NOT NULL CHECK (octet_length(project_owner) = 32),
    project_d_tag TEXT NOT NULL,
    base_event_id BYTEA NOT NULL CHECK (octet_length(base_event_id) = 32),
    revision_event_id BYTEA NOT NULL CHECK (octet_length(revision_event_id) = 32),
    related_channel_ids UUID[] NOT NULL DEFAULT '{}',
    PRIMARY KEY (community_id, project_owner, project_d_tag)
);

SELECT attach_community_write_fence('project_revision_heads');
