-- ── Search outbox ─────────────────────────────────────────────────────────────
-- Keys of events whose search index entry must be brought up to date. Written
-- in the transaction that admits the event, only when the relay is configured
-- with an OpenSearch address; the indexer claims rows with SKIP LOCKED,
-- re-reads each event from `events`, writes the index and deletes the rows.
-- Not deduplicated: a change made while an older row is claimed keeps its own
-- row. Keep this table out of logical-replication publications; its rows are
-- insert-then-delete churn that change-data-capture consumers never need.

CREATE TABLE search_outbox (
    community_id     UUID NOT NULL,
    id               UUID NOT NULL DEFAULT gen_random_uuid(),
    event_id         BYTEA NOT NULL CHECK (length(event_id) = 32),
    event_created_at TIMESTAMPTZ NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, id)
);
CREATE INDEX search_outbox_queue ON search_outbox (created_at);
CREATE INDEX search_outbox_community_queue ON search_outbox (community_id, created_at);
