-- Bounded delivery, retry, quarantine, dead-letter, and capacity state.
-- Mutable pipeline state is never stored in immutable event rows.

CREATE TABLE authorization_evidence_capacity_state (
    community_id          UUID NOT NULL REFERENCES communities(id),
    general_remaining     BIGINT NOT NULL DEFAULT 100000 CHECK (general_remaining >= 0),
    allow_reserve         BIGINT NOT NULL DEFAULT 10000 CHECK (
        allow_reserve BETWEEN 0 AND 100000
    ),
    restrictive_remaining BIGINT NOT NULL DEFAULT 10000 CHECK (restrictive_remaining >= 0),
    revision              BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id)
);

CREATE TABLE authorization_audit_outbox_delivery (
    community_id        UUID NOT NULL,
    event_id            UUID NOT NULL,
    capacity_class      SMALLINT NOT NULL CHECK (capacity_class IN (1, 2, 3)),
    delivery_state      TEXT NOT NULL DEFAULT 'pending' CHECK (
        delivery_state IN ('pending', 'leased', 'exported', 'quarantined')
    ),
    attempt_count       INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    delivery_attempt_id UUID,
    lease_owner         UUID,
    lease_expires_at    TIMESTAMPTZ,
    next_attempt_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    acknowledged_at     TIMESTAMPTZ,
    last_control_code   SMALLINT CHECK (last_control_code BETWEEN 1 AND 8),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, event_id),
    FOREIGN KEY (community_id, event_id)
        REFERENCES authorization_audit_outbox (community_id, event_id)
        ON DELETE RESTRICT,
    CHECK ((delivery_state = 'leased') =
           (delivery_attempt_id IS NOT NULL AND lease_owner IS NOT NULL
            AND lease_expires_at IS NOT NULL)),
    CHECK ((delivery_state = 'exported') = (acknowledged_at IS NOT NULL))
);

CREATE INDEX idx_authorization_audit_delivery_claim
    ON authorization_audit_outbox_delivery
       (community_id, delivery_state, next_attempt_at)
    WHERE delivery_state IN ('pending', 'leased');

CREATE TABLE authorization_decision_delivery (
    community_id        UUID NOT NULL,
    event_id            UUID NOT NULL,
    capacity_class      SMALLINT NOT NULL CHECK (capacity_class IN (1, 2, 3)),
    delivery_state      TEXT NOT NULL DEFAULT 'pending' CHECK (
        delivery_state IN ('pending', 'leased', 'exported', 'quarantined')
    ),
    attempt_count       INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    delivery_attempt_id UUID,
    lease_owner         UUID,
    lease_expires_at    TIMESTAMPTZ,
    next_attempt_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    acknowledged_at     TIMESTAMPTZ,
    last_control_code   SMALLINT CHECK (last_control_code BETWEEN 1 AND 8),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, event_id),
    FOREIGN KEY (community_id, event_id)
        REFERENCES authorization_decision_events (community_id, event_id)
        ON DELETE RESTRICT,
    CHECK ((delivery_state = 'leased') =
           (delivery_attempt_id IS NOT NULL AND lease_owner IS NOT NULL
            AND lease_expires_at IS NOT NULL)),
    CHECK ((delivery_state = 'exported') = (acknowledged_at IS NOT NULL))
);

CREATE INDEX idx_authorization_decision_delivery_claim
    ON authorization_decision_delivery
       (community_id, delivery_state, next_attempt_at)
    WHERE delivery_state IN ('pending', 'leased');

CREATE TABLE authorization_evidence_dead_letters (
    community_id        UUID NOT NULL REFERENCES communities(id),
    observation_id      UUID NOT NULL,
    audit_event_id      UUID,
    decision_event_id   UUID,
    delivery_attempt_id UUID NOT NULL,
    control_code        SMALLINT NOT NULL CHECK (control_code BETWEEN 1 AND 8),
    observed_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, observation_id),
    FOREIGN KEY (community_id, audit_event_id)
        REFERENCES authorization_audit_outbox (community_id, event_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (community_id, decision_event_id)
        REFERENCES authorization_decision_events (community_id, event_id)
        ON DELETE RESTRICT,
    CHECK ((audit_event_id IS NOT NULL)::INTEGER
         + (decision_event_id IS NOT NULL)::INTEGER = 1)
);

CREATE TABLE authorization_evidence_segment_manifests (
    community_id     UUID NOT NULL REFERENCES communities(id),
    manifest_id      UUID NOT NULL,
    stream_id        UUID NOT NULL,
    first_position   BIGINT NOT NULL CHECK (first_position > 0),
    last_position    BIGINT NOT NULL CHECK (last_position >= first_position),
    first_digest     BYTEA NOT NULL CHECK (octet_length(first_digest) = 32),
    terminal_digest  BYTEA NOT NULL CHECK (octet_length(terminal_digest) = 32),
    retention_digest BYTEA NOT NULL CHECK (octet_length(retention_digest) = 32),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, manifest_id),
    UNIQUE (community_id, stream_id, first_position, last_position),
    FOREIGN KEY (community_id, stream_id)
        REFERENCES authorization_evidence_stream_heads (community_id, stream_id)
        ON DELETE RESTRICT
);

CREATE TABLE authorization_evidence_restorations (
    community_id        UUID NOT NULL REFERENCES communities(id),
    restoration_id     UUID NOT NULL,
    audit_event_id      UUID,
    decision_event_id   UUID,
    prior_delivery_attempt_id UUID NOT NULL,
    actor_reference     BYTEA NOT NULL CHECK (octet_length(actor_reference) = 32),
    control_code        SMALLINT NOT NULL CHECK (control_code BETWEEN 1 AND 8),
    restored_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, restoration_id),
    FOREIGN KEY (community_id, audit_event_id)
        REFERENCES authorization_audit_outbox (community_id, event_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (community_id, decision_event_id)
        REFERENCES authorization_decision_events (community_id, event_id)
        ON DELETE RESTRICT,
    CHECK ((audit_event_id IS NOT NULL)::INTEGER
         + (decision_event_id IS NOT NULL)::INTEGER = 1)
);

CREATE TRIGGER authorization_evidence_dead_letters_immutable
    BEFORE UPDATE OR DELETE ON authorization_evidence_dead_letters
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();

CREATE TRIGGER authorization_evidence_dead_letters_no_truncate
    BEFORE TRUNCATE ON authorization_evidence_dead_letters
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();

CREATE TRIGGER authorization_evidence_segment_manifests_immutable
    BEFORE UPDATE OR DELETE ON authorization_evidence_segment_manifests
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();

CREATE TRIGGER authorization_evidence_segment_manifests_no_truncate
    BEFORE TRUNCATE ON authorization_evidence_segment_manifests
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();

CREATE TRIGGER authorization_evidence_restorations_immutable
    BEFORE UPDATE OR DELETE ON authorization_evidence_restorations
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();

CREATE TRIGGER authorization_evidence_restorations_no_truncate
    BEFORE TRUNCATE ON authorization_evidence_restorations
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();
