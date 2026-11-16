-- Community-owned recipient sets. Deleted IDs remain reserved until community purge.
-- Current signed heads, including tombstones, must survive any future event retention.
CREATE TABLE mention_groups (
    community_id UUID NOT NULL,
    id UUID NOT NULL CHECK (id <> '00000000-0000-0000-0000-000000000000'::uuid),
    name TEXT NOT NULL,
    members TEXT[] NOT NULL,
    channels UUID[] NOT NULL DEFAULT '{}' CHECK (cardinality(channels) <= 32),
    version BIGINT NOT NULL CHECK (version > 0),
    deleted_at TIMESTAMPTZ,
    state_event_id BYTEA NOT NULL CHECK (length(state_event_id) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, id)
);
CREATE UNIQUE INDEX idx_mention_groups_live_name
    ON mention_groups (community_id, lower(name)) WHERE deleted_at IS NULL;
