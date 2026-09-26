-- Private accessory read progress. Never included in Nostr event queries.
-- Frontiers use signed event time. An empty root_id covers only the channel
-- timeline; a root-specific frontier covers that thread, without inheritance.
CREATE TABLE personal_read_accounts (
    community_id UUID NOT NULL REFERENCES communities(id),
    actor BYTEA NOT NULL CHECK (octet_length(actor) = 32),
    imported_at TIMESTAMPTZ,
    PRIMARY KEY (community_id, actor)
);

CREATE TABLE personal_read_frontiers (
    community_id UUID NOT NULL,
    actor BYTEA NOT NULL,
    channel_id UUID NOT NULL,
    root_id BYTEA NOT NULL DEFAULT ''::bytea CHECK (octet_length(root_id) IN (0, 32)),
    through_timestamp BIGINT NOT NULL CHECK (through_timestamp >= 0),
    PRIMARY KEY (community_id, actor, channel_id, root_id),
    FOREIGN KEY (community_id, actor)
        REFERENCES personal_read_accounts (community_id, actor) ON DELETE CASCADE,
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);



SELECT attach_community_write_fence('personal_read_accounts');
SELECT attach_community_write_fence('personal_read_frontiers');
