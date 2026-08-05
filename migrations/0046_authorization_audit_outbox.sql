-- Transactional, append-only authorization audit outbox.
--
-- Immutable evidence is separated from delivery state. Stream positions are
-- allocated while holding the stream-head row lock. No column accepts raw
-- identity, credentials, arbitrary JSON, or unbounded policy data.

CREATE TABLE authorization_evidence_stream_heads (
    community_id    UUID NOT NULL REFERENCES communities(id),
    stream_kind     SMALLINT NOT NULL CHECK (stream_kind IN (1, 2, 3)),
    stream_id       UUID NOT NULL,
    next_position   BIGINT NOT NULL DEFAULT 1 CHECK (next_position > 0),
    terminal_digest BYTEA NOT NULL DEFAULT decode(repeat('00', 32), 'hex')
                    CHECK (octet_length(terminal_digest) = 32),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, stream_kind),
    UNIQUE (community_id, stream_id)
);

CREATE TABLE authorization_evidence_event_registry (
    community_id   UUID NOT NULL REFERENCES communities(id),
    event_id       UUID NOT NULL,
    stream_kind    SMALLINT NOT NULL CHECK (stream_kind IN (1, 2)),
    content_digest BYTEA NOT NULL CHECK (octet_length(content_digest) = 32),
    registered_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, event_id)
);

CREATE TABLE authorization_audit_outbox (
    community_id        UUID NOT NULL REFERENCES communities(id),
    event_id            UUID NOT NULL,
    stream_id           UUID NOT NULL,
    stream_position     BIGINT NOT NULL CHECK (stream_position > 0),
    schema_version      SMALLINT NOT NULL CHECK (schema_version = 1),
    occurred_at         TIMESTAMPTZ NOT NULL,
    accepted_at         TIMESTAMPTZ NOT NULL,
    operation_id        UUID,
    correlation_id      UUID NOT NULL,
    attempt_id          UUID NOT NULL,
    event_kind          SMALLINT NOT NULL CHECK (event_kind BETWEEN 1 AND 64),
    event_result        SMALLINT NOT NULL CHECK (event_result BETWEEN 1 AND 8),
    decision_reason     SMALLINT NOT NULL CHECK (decision_reason BETWEEN 1 AND 38),
    actor_class         SMALLINT NOT NULL CHECK (actor_class BETWEEN 1 AND 6),
    canonical_event     BYTEA NOT NULL CHECK (
        octet_length(canonical_event) BETWEEN 1 AND 65536
    ),
    content_digest      BYTEA NOT NULL CHECK (octet_length(content_digest) = 32),
    previous_digest     BYTEA NOT NULL CHECK (octet_length(previous_digest) = 32),
    chain_digest        BYTEA NOT NULL CHECK (octet_length(chain_digest) = 32),
    PRIMARY KEY (community_id, event_id),
    UNIQUE (community_id, stream_id, stream_position),
    FOREIGN KEY (community_id, stream_id)
        REFERENCES authorization_evidence_stream_heads (community_id, stream_id)
        ON DELETE RESTRICT
);

CREATE FUNCTION authorization_immutable_row_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'authorization evidence rows are append-only'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;

CREATE TRIGGER authorization_audit_outbox_immutable
    BEFORE UPDATE OR DELETE ON authorization_audit_outbox
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();

CREATE TRIGGER authorization_audit_outbox_no_truncate
    BEFORE TRUNCATE ON authorization_audit_outbox
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();

CREATE TRIGGER authorization_evidence_event_registry_immutable
    BEFORE UPDATE OR DELETE ON authorization_evidence_event_registry
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();

CREATE TRIGGER authorization_evidence_event_registry_no_truncate
    BEFORE TRUNCATE ON authorization_evidence_event_registry
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();
