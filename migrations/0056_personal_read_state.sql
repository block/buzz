-- Private accessory read progress. Never included in Nostr event queries.
-- A frontier is the relay arrival time (events.received_at) of the message a
-- context was read through, never signed event time, which the sender chooses.
-- An empty root_id covers only the channel timeline; a root-specific frontier
-- covers that thread, without inheritance.
-- threads_through_timestamp is the only cross-context cut: an explicit
-- whole-channel read that also covers every thread in that channel.
-- started_at is the actor's first read intent. NULL until then: an account
-- can exist before its actor has started.
CREATE TABLE personal_read_accounts (
    community_id UUID NOT NULL REFERENCES communities(id),
    actor BYTEA NOT NULL CHECK (octet_length(actor) = 32),
    started_at TIMESTAMPTZ,
    PRIMARY KEY (community_id, actor)
);

CREATE TABLE personal_read_frontiers (
    community_id UUID NOT NULL,
    actor BYTEA NOT NULL,
    channel_id UUID NOT NULL,
    root_id BYTEA NOT NULL DEFAULT ''::bytea CHECK (octet_length(root_id) IN (0, 32)),
    through_timestamp TIMESTAMPTZ NOT NULL,
    -- Whole-channel cut covering every thread; channel rows only.
    threads_through_timestamp TIMESTAMPTZ
        CHECK (threads_through_timestamp IS NULL OR root_id = ''::bytea),
    PRIMARY KEY (community_id, actor, channel_id, root_id),
    FOREIGN KEY (community_id, actor)
        REFERENCES personal_read_accounts (community_id, actor) ON DELETE CASCADE,
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);



SELECT attach_community_write_fence('personal_read_accounts');
SELECT attach_community_write_fence('personal_read_frontiers');
