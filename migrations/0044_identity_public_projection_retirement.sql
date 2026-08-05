-- Durable, provider-neutral reconciliation for the optional public identity
-- projection. O3 lifecycle rows remain the authority; these tables contain
-- only public event coordinates and opaque binding generations.

CREATE TABLE identity_public_projection_heads (
    community_id          UUID NOT NULL REFERENCES communities(id),
    relay_pubkey          BYTEA NOT NULL,
    subject_pubkey        BYTEA NOT NULL,
    event_id              BYTEA NOT NULL,
    event_created_at      TIMESTAMPTZ NOT NULL,
    disposition           TEXT NOT NULL,
    source_binding_id     UUID,
    source_binding_version BIGINT,
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, relay_pubkey, subject_pubkey),
    FOREIGN KEY (community_id, source_binding_id)
        REFERENCES identity_bindings (community_id, binding_id),
    CHECK (length(relay_pubkey) = 32),
    CHECK (length(subject_pubkey) = 32),
    CHECK (length(event_id) = 32),
    CHECK (disposition IN ('active', 'inactive')),
    CHECK (source_binding_version IS NULL OR source_binding_version > 0),
    CHECK (
        (source_binding_id IS NULL AND source_binding_version IS NULL)
        OR
        (source_binding_id IS NOT NULL AND source_binding_version IS NOT NULL)
    )
);

CREATE TABLE identity_public_projection_retirements (
    community_id          UUID NOT NULL REFERENCES communities(id),
    operation_id          UUID NOT NULL,
    relay_pubkey          BYTEA NOT NULL,
    old_pubkey            BYTEA NOT NULL,
    source_binding_id     UUID,
    source_binding_version BIGINT,
    operation_kind        TEXT NOT NULL,
    phase                 TEXT NOT NULL DEFAULT 'projection',
    outcome               TEXT,
    event_id              BYTEA,
    attempts              BIGINT NOT NULL DEFAULT 0,
    next_attempt_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    claim_token           UUID,
    lease_until           TIMESTAMPTZ,
    completed_at          TIMESTAMPTZ,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, operation_id, relay_pubkey),
    FOREIGN KEY (community_id, operation_id)
        REFERENCES identity_lifecycle_operations (community_id, operation_id),
    FOREIGN KEY (community_id, source_binding_id)
        REFERENCES identity_bindings (community_id, binding_id),
    CHECK (length(relay_pubkey) = 32),
    CHECK (length(old_pubkey) = 32),
    CHECK (source_binding_version IS NULL OR source_binding_version > 0),
    CHECK (
        (source_binding_id IS NULL AND source_binding_version IS NULL)
        OR
        (source_binding_id IS NOT NULL AND source_binding_version IS NOT NULL)
    ),
    CHECK (operation_kind IN ('revoke_key', 'rotate')),
    CHECK (phase IN ('projection', 'delivery', 'completed', 'superseded')),
    CHECK (outcome IS NULL OR outcome IN (
        'no_projection', 'already_inactive', 'replaced_inactive',
        'newer_binding', 'newer_projection'
    )),
    CHECK (event_id IS NULL OR length(event_id) = 32),
    CHECK (attempts >= 0),
    CHECK ((claim_token IS NULL) = (lease_until IS NULL)),
    CHECK (
        (phase IN ('completed', 'superseded') AND completed_at IS NOT NULL)
        OR
        (phase IN ('projection', 'delivery') AND completed_at IS NULL)
    )
);

CREATE INDEX idx_identity_public_projection_retirements_ready
    ON identity_public_projection_retirements
        (phase, next_attempt_at, community_id, operation_id)
    WHERE phase IN ('projection', 'delivery');
