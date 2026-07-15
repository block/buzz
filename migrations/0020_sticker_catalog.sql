-- Workspace sticker-pack curation is revision-pinned. A coordinate alone is
-- not sufficient authority because its kind:30031 head can be replaced by its
-- author after approval.
CREATE TABLE sticker_catalog_approvals (
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    coordinate TEXT NOT NULL CHECK (octet_length(coordinate) BETWEEN 72 AND 151),
    approved_event_id BYTEA NOT NULL CHECK (length(approved_event_id) = 32),
    approved_by BYTEA NOT NULL CHECK (length(approved_by) = 32),
    approved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, coordinate)
);

CREATE INDEX idx_sticker_catalog_approvals_event
    ON sticker_catalog_approvals (community_id, approved_event_id);
