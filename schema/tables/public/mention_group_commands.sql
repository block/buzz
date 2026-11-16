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
