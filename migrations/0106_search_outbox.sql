-- The search indexer's queue; see schema/tables/public/search_outbox.sql.
-- Its writers use community admission (docs/database.md). A new, empty table.
SET LOCAL lock_timeout = '5s';
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
