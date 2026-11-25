-- ── Search backfill ───────────────────────────────────────────────────────────
-- How far the search indexer has walked each community's searchable events,
-- newest first, queuing their keys in `search_outbox`. The cursor is the
-- (created_at, id) key of the oldest event queued so far; NULL before the
-- first batch. `completed_at` is set once no older event is left. Deleting a
-- community's row backfills it again from its newest event. Keep this table
-- out of logical-replication publications, like `search_outbox`.

CREATE TABLE search_backfill (
    community_id      UUID NOT NULL,
    cursor_created_at TIMESTAMPTZ,
    cursor_event_id   BYTEA CHECK (length(cursor_event_id) = 32),
    enqueued          BIGINT NOT NULL DEFAULT 0,
    completed_at      TIMESTAMPTZ,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id)
);
