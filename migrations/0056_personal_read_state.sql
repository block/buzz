-- Private accessory read progress. Never included in Nostr event queries.
-- A frontier is the relay arrival time (events.received_at) of the message a
-- context was read through, never signed event time, which the sender chooses.
-- An empty root_id covers only the channel timeline; a root-specific frontier
-- covers that thread, without inheritance. A thread row exists for each
-- thread the actor follows or has unfollowed. Ingest moves the author's own
-- frontiers through each message they post.
-- threads_through_timestamp is the only cross-context cut: an explicit
-- whole-channel read that also covers every thread in that channel.
-- started_at is the actor's first read intent and the floor of every
-- position: until then nothing counts, and from then the actor starts caught
-- up. Ingest creates accounts for thread membership without starting them.
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
    -- Thread rows only: false after unfollow, kept so later replies don't
    -- re-follow. Replying or a follow intent sets it again.
    following BOOLEAN NOT NULL DEFAULT true CHECK (following OR root_id <> ''::bytea),
    PRIMARY KEY (community_id, actor, channel_id, root_id),
    FOREIGN KEY (community_id, actor)
        REFERENCES personal_read_accounts (community_id, actor) ON DELETE CASCADE,
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);


-- Arrival time of a thread root's latest reply: the sidebar skips threads
-- with nothing new since the reader's position without reading replies.
ALTER TABLE thread_metadata ADD COLUMN last_reply_received_at TIMESTAMPTZ;

-- Arrival time and read scope of each mention, so the sidebar counts a
-- reader's unread mentions in one channel timeline or thread from its
-- position. root_id is the thread a reply belongs to, NULL when the mention
-- shows on the channel timeline (a top-level message or broadcast reply).
-- Existing rows stay NULL: every position is floored at its account's
-- started_at, set by a read intent after this migration, so no earlier
-- mention can be unread.
ALTER TABLE event_mentions ADD COLUMN received_at TIMESTAMPTZ, ADD COLUMN root_id BYTEA;
CREATE INDEX idx_event_mentions_scope_received
    ON event_mentions (community_id, pubkey_hex, channel_id, root_id, received_at);

SELECT attach_community_write_fence('personal_read_accounts');
SELECT attach_community_write_fence('personal_read_frontiers');
