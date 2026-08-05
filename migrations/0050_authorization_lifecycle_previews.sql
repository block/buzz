-- Immutable, bounded rotation-preview evidence.

CREATE TABLE authorization_lifecycle_previews (
    community_id       UUID NOT NULL REFERENCES communities(id),
    preview_digest     BYTEA NOT NULL CHECK (octet_length(preview_digest) = 32),
    operation_id       UUID NOT NULL,
    target_reference   BYTEA NOT NULL CHECK (octet_length(target_reference) = 32),
    replacement_reference BYTEA NOT NULL CHECK (octet_length(replacement_reference) = 32),
    lifecycle_revision BIGINT NOT NULL CHECK (lifecycle_revision > 0),
    affected_count     INTEGER NOT NULL CHECK (affected_count BETWEEN 0 AND 100),
    expires_at         TIMESTAMPTZ NOT NULL,
    decision_event_id  UUID NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, preview_digest),
    UNIQUE (community_id, operation_id),
    FOREIGN KEY (community_id, decision_event_id)
        REFERENCES authorization_decision_events (community_id, event_id)
        ON DELETE RESTRICT
);

CREATE INDEX idx_authorization_lifecycle_previews_expiry
    ON authorization_lifecycle_previews (community_id, expires_at);

CREATE TRIGGER authorization_lifecycle_previews_immutable
    BEFORE UPDATE OR DELETE ON authorization_lifecycle_previews
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();

CREATE TRIGGER authorization_lifecycle_previews_no_truncate
    BEFORE TRUNCATE ON authorization_lifecycle_previews
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();
