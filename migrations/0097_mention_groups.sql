-- Compatibility prerequisite: deploy the pending-table catalog release everywhere first.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

ALTER TABLE communities
    ADD COLUMN mention_groups_manage TEXT NOT NULL DEFAULT 'everyone' CHECK (mention_groups_manage IN ('admins', 'everyone')),
    ADD COLUMN mention_groups_version BIGINT NOT NULL DEFAULT 0 CHECK (mention_groups_version >= 0),
    ADD COLUMN mention_groups_state_event_id BYTEA CHECK (length(mention_groups_state_event_id) = 32);

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

-- Durable accepted commands, independent of event retirement and head changes.
CREATE TABLE mention_group_commands (
    community_id UUID NOT NULL,
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    signer TEXT NOT NULL,
    group_id UUID,
    operation TEXT NOT NULL CHECK (operation IN ('put', 'delete', 'policy')),
    version BIGINT NOT NULL CHECK (version > 0),
    state_event_id BYTEA NOT NULL CHECK (length(state_event_id) = 32),
    command JSONB NOT NULL,
    join_summary JSONB NOT NULL,
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, event_id),
    CHECK ((operation = 'policy') = (group_id IS NULL))
);
