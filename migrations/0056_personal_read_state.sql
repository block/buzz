-- Private accessory read progress. Never included in Nostr event queries.
--
-- Unread is counted by subtraction over relay-assigned message numbers.
-- Ingest numbers every eligible message in its channel under one locked
-- counter row:
--   events.channel_seq   every eligible message in the channel, in commit order;
--   events.timeline_seq  the channel's timeline count once the message is
--                        stored (its own number when it is on the timeline);
--   thread_metadata.reply_seq on a root: the thread's eligible replies so far,
--                        broadcast depth-1 replies excluded (they count on the
--                        timeline), with reply_channel_seq and reply_last_id
--                        the latest one's.
-- A position is the numbers a context was read through. Unread is the latest
-- number minus the position, so every numbered message counts for every
-- reader: posting marks the author's position, and deleted messages count
-- until read past.
--
-- started_at is the actor's first read intent: until then nothing counts.
-- Starting writes a caught-up position for every joined channel and thread,
-- and joining a channel afterwards writes one at its current count.
CREATE TABLE personal_read_accounts (
    community_id UUID NOT NULL REFERENCES communities(id),
    actor BYTEA NOT NULL CHECK (octet_length(actor) = 32),
    started_at TIMESTAMPTZ,
    PRIMARY KEY (community_id, actor)
);

CREATE TABLE personal_read_counters (
    community_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    timeline_seq BIGINT NOT NULL DEFAULT 0,
    channel_seq BIGINT NOT NULL DEFAULT 0,
    -- The message numbered channel_seq: marking the channel read through it
    -- reads everything, so a badge has an anchor without a scan.
    latest_id BYTEA,
    PRIMARY KEY (community_id, channel_id),
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);

-- An empty root_id is the channel timeline. A thread row exists exactly for
-- the actor's threads. through_seq is in the context's own numbering
-- (timeline or thread); through_channel_seq is the channel-wide cut it was
-- read through. through_timestamp is the anchor's arrival, used only to
-- bound the mention scan.
CREATE TABLE personal_read_frontiers (
    community_id UUID NOT NULL,
    actor BYTEA NOT NULL,
    channel_id UUID NOT NULL,
    root_id BYTEA NOT NULL DEFAULT ''::bytea CHECK (octet_length(root_id) IN (0, 32)),
    through_seq BIGINT NOT NULL,
    through_channel_seq BIGINT NOT NULL,
    through_timestamp TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, actor, channel_id, root_id),
    FOREIGN KEY (community_id, actor)
        REFERENCES personal_read_accounts (community_id, actor) ON DELETE CASCADE,
    FOREIGN KEY (community_id, channel_id)
        REFERENCES channels (community_id, id) ON DELETE CASCADE
);

ALTER TABLE events ADD COLUMN channel_seq BIGINT, ADD COLUMN timeline_seq BIGINT;
ALTER TABLE thread_metadata ADD COLUMN reply_seq BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN reply_channel_seq BIGINT, ADD COLUMN reply_last_id BYTEA;

-- Joining (or rejoining) a channel after starting begins caught up.
CREATE FUNCTION personal_read_on_join() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.removed_at IS NULL AND (TG_OP = 'INSERT' OR OLD.removed_at IS NOT NULL) THEN
        INSERT INTO personal_read_frontiers (community_id, actor, channel_id,
            through_seq, through_channel_seq, through_timestamp)
        SELECT NEW.community_id, NEW.pubkey, NEW.channel_id,
            COALESCE(k.timeline_seq, 0), COALESCE(k.channel_seq, 0), now()
        FROM personal_read_accounts a
        LEFT JOIN personal_read_counters k
            ON k.community_id = NEW.community_id AND k.channel_id = NEW.channel_id
        WHERE a.community_id = NEW.community_id AND a.actor = NEW.pubkey
            AND a.started_at IS NOT NULL
        ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE SET
            through_seq = EXCLUDED.through_seq,
            through_channel_seq = EXCLUDED.through_channel_seq,
            through_timestamp = EXCLUDED.through_timestamp;
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER trg_channel_members_personal_read_join
    AFTER INSERT OR UPDATE OF removed_at ON channel_members
    FOR EACH ROW EXECUTE FUNCTION personal_read_on_join();

SELECT attach_community_write_fence('personal_read_accounts');
SELECT attach_community_write_fence('personal_read_counters');
SELECT attach_community_write_fence('personal_read_frontiers');
