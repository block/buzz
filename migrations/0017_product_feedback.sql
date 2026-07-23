-- Buzz product feedback is accepted through a dedicated signed event kind and
-- sidecarred here instead of entering the ordinary events table. Rows remain
-- attributable to their source community, while deployment operators may
-- review the table across communities through internal tooling.
CREATE TABLE product_feedback (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    community_id UUID NOT NULL REFERENCES communities(id),
    event_id BYTEA NOT NULL CHECK (length(event_id) = 32),
    submitter_pubkey BYTEA NOT NULL CHECK (length(submitter_pubkey) = 32),
    category TEXT CHECK (category IN ('bug', 'praise', 'needs-work')),
    body TEXT NOT NULL CHECK (length(btrim(body)) > 0),
    tags JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(tags) = 'array'),
    event_created_at TIMESTAMPTZ NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (event_id)
);

CREATE INDEX idx_product_feedback_received
    ON product_feedback (received_at DESC, id);
CREATE INDEX idx_product_feedback_community_received
    ON product_feedback (community_id, received_at DESC, id);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('product_feedback', 'deployment product inbox; community_id is provenance only');
