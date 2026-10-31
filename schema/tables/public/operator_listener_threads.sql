CREATE TABLE operator_listener_threads (
    community_id UUID NOT NULL,
    listener_pubkey BYTEA NOT NULL CHECK (length(listener_pubkey) = 32),
    target_pubkey BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    channel_id UUID NOT NULL,
    root_event_id BYTEA NOT NULL CHECK (length(root_event_id) = 32),
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, listener_pubkey, target_pubkey, channel_id, root_event_id)
);
CREATE INDEX operator_listener_threads_match
    ON operator_listener_threads (community_id, channel_id, root_event_id, expires_at);

CREATE INDEX operator_listener_threads_expiry ON operator_listener_threads (expires_at, community_id);
