-- Private accessory read progress. Never included in Nostr event queries.
-- A frontier is the relay arrival time (events.received_at) of the message a
-- context was read through; the unread cutoff alone is signed event time. An
-- empty root_id denotes a channel frontier.
-- started_at is the actor's first read intent. NULL until then: an account
-- can exist before its actor has started.
CREATE TABLE personal_read_accounts (
    community_id UUID NOT NULL REFERENCES communities(id),
    actor BYTEA NOT NULL CHECK (octet_length(actor) = 32),
    started_at TIMESTAMPTZ,
    PRIMARY KEY (community_id, actor)
);
