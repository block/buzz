-- Per-community search backfill progress; see
-- schema/tables/public/search_backfill.sql. Its writers use community
-- admission (docs/database.md). A new, empty table.
SET LOCAL lock_timeout = '5s';
CREATE TABLE search_backfill (
    community_id      UUID NOT NULL,
    cursor_created_at TIMESTAMPTZ,
    cursor_event_id   BYTEA CHECK (length(cursor_event_id) = 32),
    enqueued          BIGINT NOT NULL DEFAULT 0,
    completed_at      TIMESTAMPTZ,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id)
);
