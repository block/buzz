-- Runtime-gated by BUZZ_EXPERIMENTAL_INTERACTIONS (off by default).
-- Signed events remain in events; this table serializes one prompt's transitions.
CREATE TABLE interactions (
    community_id UUID NOT NULL REFERENCES communities(id),
    prompt_id BYTEA NOT NULL CHECK (octet_length(prompt_id) = 32),
    channel_id UUID NOT NULL,
    author BYTEA NOT NULL,
    prompt JSONB NOT NULL,
    state JSONB NOT NULL,
    projection_id BYTEA NOT NULL,
    state_timestamp BIGINT NOT NULL,
    expiration BIGINT NOT NULL,
    closed BOOLEAN NOT NULL DEFAULT FALSE,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, prompt_id),
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id) ON DELETE CASCADE
);
CREATE INDEX idx_interactions_open ON interactions (expiration) WHERE NOT closed;
CREATE INDEX idx_interactions_channel ON interactions (community_id, channel_id) WHERE NOT closed;
CREATE INDEX idx_interactions_author ON interactions (community_id, author, received_at);
CREATE UNIQUE INDEX idx_interactions_projection ON interactions (community_id, projection_id);

-- Durable at-least-once Redis delivery. Repeated event IDs are harmless to clients.
CREATE TABLE interaction_outbox (
    community_id UUID NOT NULL REFERENCES communities(id),
    event_id BYTEA NOT NULL,
    channel_id UUID NOT NULL,
    event JSONB NOT NULL,
    queued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, event_id),
    FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id) ON DELETE CASCADE
);
CREATE INDEX idx_interaction_outbox_queued ON interaction_outbox (queued_at);
