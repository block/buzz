-- Monotonic per-community authority for protected Git and media visibility.
--
-- Missing rows are treated as legacy only by migration-aware binaries. Once a
-- row enters `importing`, writes to the legacy visibility object are fenced and
-- the state can advance only to `postgresql`.

CREATE TABLE protected_object_authority (
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    surface TEXT NOT NULL CHECK (surface IN ('git', 'media')),
    state TEXT NOT NULL CHECK (state IN ('legacy', 'importing', 'postgresql')),
    generation BIGINT NOT NULL CHECK (generation > 0),
    imported_objects BIGINT NOT NULL DEFAULT 0 CHECK (imported_objects >= 0),
    inventory_sha256 TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, surface),
    CHECK (inventory_sha256 IS NULL OR inventory_sha256 ~ '^[0-9a-f]{64}$'),
    CHECK (
        (state = 'legacy' AND started_at IS NULL AND completed_at IS NULL)
        OR (state = 'importing' AND started_at IS NOT NULL AND completed_at IS NULL)
        OR (state = 'postgresql' AND started_at IS NOT NULL AND completed_at IS NOT NULL
            AND inventory_sha256 IS NOT NULL)
    )
);

CREATE INDEX idx_protected_object_authority_state
    ON protected_object_authority (state, community_id, surface);
