-- Append-only evidence for non-mutating authorization decisions.

CREATE TABLE authorization_decision_events (
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

CREATE TRIGGER authorization_decision_events_immutable
    BEFORE UPDATE OR DELETE ON authorization_decision_events
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();

CREATE TRIGGER authorization_decision_events_no_truncate
    BEFORE TRUNCATE ON authorization_decision_events
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();
