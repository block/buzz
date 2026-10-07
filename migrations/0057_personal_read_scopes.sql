-- Channel and thread read scopes on top of 0056 (checksum-frozen).
-- Startup is bounded like 0049: this migration takes exclusive locks on
-- thread_metadata, channels and event_mentions, and must fail rather than
-- queue ingestion behind a long reader or an index build on a populated
-- table. Brownfield operators prebuild idx_event_mentions_scope_received and
-- pre-drop idx_thread_metadata_root as documented in
-- docs/buzz-v1-read-state.md.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

-- started_at is the actor's first read intent and the floor of every
-- position: until then nothing counts, and from then the actor starts caught
-- up. Ingest creates accounts for thread membership without starting them.
-- Accounts from 0056 stay NULL and start at their next read intent.
ALTER TABLE personal_read_accounts ADD COLUMN started_at TIMESTAMPTZ;

-- A thread row exists for each thread the actor follows or has unfollowed.
-- Ingest moves the author's own frontiers through each message they post.
-- following: thread rows only, false after unfollow, kept so later replies
-- don't re-follow. Replying or a follow intent sets it again.
ALTER TABLE personal_read_frontiers
    ADD COLUMN following BOOLEAN NOT NULL DEFAULT true
        CHECK (following OR root_id <> ''::bytea);

-- Arrival time of a thread root's latest reply: the sidebar skips threads
-- with nothing new since the reader's position without reading replies.
ALTER TABLE thread_metadata ADD COLUMN last_reply_received_at TIMESTAMPTZ;

-- Arrival time of a channel's latest eligible timeline message (top-level or
-- broadcast reply): the sidebar answers "anything new?" for a caught-up
-- reader with one comparison instead of walking past replies. Unindexed so
-- updates stay HOT. NULL for channels with no timeline message since this
-- migration, which no position can be behind: every position is floored at
-- a started_at set after it.
ALTER TABLE channels ADD COLUMN last_timeline_received_at TIMESTAMPTZ;

-- Arrival time and read scope of each mention, so the sidebar counts a
-- reader's unread mentions in one channel timeline or thread from its
-- position. root_id is the thread a reply belongs to, NULL when the mention
-- shows on the channel timeline (a top-level message or broadcast reply).
-- Existing rows stay NULL: every position is floored at its account's
-- started_at, set by a read intent after this migration, so no earlier
-- mention can be unread.
-- IF NOT EXISTS admits the brownfield prebuild, which adds these columns
-- before building the index concurrently. As in 0049, a prebuilt index skips
-- CREATE INDEX and must pass the catalog check below.
ALTER TABLE event_mentions
    ADD COLUMN IF NOT EXISTS received_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS root_id BYTEA;
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'event_mentions'
          AND ((column_name = 'received_at' AND data_type = 'timestamp with time zone')
            OR (column_name = 'root_id' AND data_type = 'bytea'))
        HAVING count(*) = 2
    ) THEN
        RAISE EXCEPTION 'event_mentions.received_at/root_id have the wrong type; see docs/buzz-v1-read-state.md';
    END IF;

    IF to_regclass('public.idx_event_mentions_scope_received') IS NULL THEN
        CREATE INDEX idx_event_mentions_scope_received
            ON public.event_mentions (community_id, pubkey_hex, channel_id, root_id, received_at);
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND c.relname = 'idx_event_mentions_scope_received'
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              'CREATE INDEX idx_event_mentions_scope_received ON public.event_mentions USING btree (community_id, pubkey_hex, channel_id, root_id, received_at)'
    ) THEN
        RAISE EXCEPTION 'idx_event_mentions_scope_received invalid or wrong definition; see docs/buzz-v1-read-state.md';
    END IF;
END $$;

-- idx_thread_metadata_window (0049) leads with the same columns. With both,
-- the planner can take this narrower index for a thread's latest reply and
-- then sort every reply in the thread instead of reading the first row.
-- This supersedes 0049's "legacy root/parent indexes remain" for the root
-- index (0049 is checksum-frozen); the parent index remains.
DROP INDEX IF EXISTS idx_thread_metadata_root;
