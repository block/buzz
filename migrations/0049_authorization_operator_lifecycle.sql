-- Authenticated, idempotent operator lifecycle receipts and effects.
--
-- The stock relay does not register the corresponding routes. These tables
-- retain only pseudonymous or access-controlled references and closed fields.

CREATE TABLE authorization_operator_lifecycle_revisions (
    community_id UUID NOT NULL REFERENCES communities(id),
    revision     BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id)
);

CREATE TABLE authorization_operator_binding_refs (
    community_id     UUID NOT NULL REFERENCES communities(id),
    binding_reference BYTEA NOT NULL CHECK (octet_length(binding_reference) = 32),
    binding_id       UUID NOT NULL,
    key_epoch        INTEGER NOT NULL CHECK (key_epoch > 0),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, binding_reference),
    UNIQUE (community_id, binding_id),
    FOREIGN KEY (community_id, binding_id)
        REFERENCES identity_bindings (community_id, binding_id)
        ON DELETE RESTRICT
);

CREATE TABLE authorization_operator_operation_receipts (
    community_id       UUID NOT NULL REFERENCES communities(id),
    operation_id       UUID NOT NULL,
    semantic_fingerprint BYTEA NOT NULL CHECK (octet_length(semantic_fingerprint) = 32),
    correlation_id     UUID NOT NULL,
    action             SMALLINT NOT NULL CHECK (action BETWEEN 1 AND 4),
    outcome_status     SMALLINT NOT NULL CHECK (outcome_status BETWEEN 1 AND 5),
    decision_reason    SMALLINT NOT NULL CHECK (decision_reason BETWEEN 1 AND 38),
    reason_code        SMALLINT NOT NULL CHECK (reason_code BETWEEN 1 AND 7),
    actor_reference    BYTEA NOT NULL CHECK (octet_length(actor_reference) = 32),
    provenance_reference BYTEA NOT NULL CHECK (octet_length(provenance_reference) = 32),
    affected_count     INTEGER NOT NULL CHECK (affected_count BETWEEN 0 AND 100),
    lifecycle_revision BIGINT NOT NULL CHECK (lifecycle_revision > 0),
    audit_event_id     UUID NOT NULL,
    committed_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, operation_id),
    FOREIGN KEY (community_id, audit_event_id)
        REFERENCES authorization_audit_outbox (community_id, event_id)
        ON DELETE RESTRICT
);

CREATE TABLE authorization_operator_result_records (
    community_id    UUID NOT NULL,
    operation_id    UUID NOT NULL,
    ordinal         SMALLINT NOT NULL CHECK (ordinal BETWEEN 0 AND 99),
    record_reference BYTEA NOT NULL CHECK (octet_length(record_reference) = 32),
    record_state    SMALLINT NOT NULL CHECK (record_state BETWEEN 1 AND 4),
    record_revision BIGINT NOT NULL CHECK (record_revision > 0),
    PRIMARY KEY (community_id, operation_id, ordinal),
    FOREIGN KEY (community_id, operation_id)
        REFERENCES authorization_operator_operation_receipts (community_id, operation_id)
        ON DELETE RESTRICT
);

CREATE TABLE authorization_operator_authority_consumptions (
    community_id      UUID NOT NULL REFERENCES communities(id),
    evidence_id       UUID NOT NULL,
    operation_id      UUID NOT NULL,
    actor_reference   BYTEA NOT NULL CHECK (octet_length(actor_reference) = 32),
    intent_digest     BYTEA NOT NULL CHECK (octet_length(intent_digest) = 32),
    evidence_expires_at TIMESTAMPTZ NOT NULL,
    consumed_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, evidence_id)
);

CREATE TABLE authorization_operator_approval_consumptions (
    community_id      UUID NOT NULL REFERENCES communities(id),
    approval_id       UUID NOT NULL,
    operation_id      UUID NOT NULL,
    approver_reference BYTEA NOT NULL CHECK (octet_length(approver_reference) = 32),
    intent_digest     BYTEA NOT NULL CHECK (octet_length(intent_digest) = 32),
    approval_expires_at TIMESTAMPTZ NOT NULL,
    consumed_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, approval_id)
);

CREATE FUNCTION authorization_operator_authority_expiry_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.evidence_expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION 'operator evidence expired before commit'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

CREATE CONSTRAINT TRIGGER authorization_operator_authority_expiry
    AFTER INSERT OR UPDATE OF evidence_expires_at
    ON authorization_operator_authority_consumptions
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operator_authority_expiry_guard();

CREATE FUNCTION authorization_operator_approval_expiry_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.approval_expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION 'operator approval expired before commit'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

CREATE CONSTRAINT TRIGGER authorization_operator_approval_expiry
    AFTER INSERT OR UPDATE OF approval_expires_at
    ON authorization_operator_approval_consumptions
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operator_approval_expiry_guard();

CREATE TABLE authorization_operator_effects (
    community_id      UUID NOT NULL REFERENCES communities(id),
    effect_id         UUID NOT NULL,
    operation_id      UUID NOT NULL,
    effect_kind       SMALLINT NOT NULL CHECK (effect_kind IN (1, 2)),
    target_reference  BYTEA NOT NULL CHECK (octet_length(target_reference) = 32),
    lifecycle_revision BIGINT NOT NULL CHECK (lifecycle_revision > 0),
    audit_event_id    UUID NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, effect_id),
    UNIQUE (community_id, operation_id),
    FOREIGN KEY (community_id, audit_event_id)
        REFERENCES authorization_audit_outbox (community_id, event_id)
        ON DELETE RESTRICT
);

ALTER TABLE identity_binding_history
    DROP CONSTRAINT identity_binding_history_transition_kind_check;
ALTER TABLE identity_binding_history
    ADD CONSTRAINT identity_binding_history_transition_kind_check CHECK (
        transition_kind IN (
            'legacy_import', 'enroll', 'provision', 'provenance_strengthened',
            'retire_pair', 'disable_identity', 'revoke_key', 'revoke_binding',
            'rotate', 'recover', 'enable_identity', 'archive'
        )
    );

CREATE TRIGGER authorization_operator_binding_refs_immutable
    BEFORE UPDATE OR DELETE ON authorization_operator_binding_refs
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_binding_refs_no_truncate
    BEFORE TRUNCATE ON authorization_operator_binding_refs
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_receipts_immutable
    BEFORE UPDATE OR DELETE ON authorization_operator_operation_receipts
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_receipts_no_truncate
    BEFORE TRUNCATE ON authorization_operator_operation_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_results_immutable
    BEFORE UPDATE OR DELETE ON authorization_operator_result_records
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_results_no_truncate
    BEFORE TRUNCATE ON authorization_operator_result_records
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_authority_immutable
    BEFORE UPDATE OR DELETE ON authorization_operator_authority_consumptions
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_authority_no_truncate
    BEFORE TRUNCATE ON authorization_operator_authority_consumptions
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_approvals_immutable
    BEFORE UPDATE OR DELETE ON authorization_operator_approval_consumptions
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_approvals_no_truncate
    BEFORE TRUNCATE ON authorization_operator_approval_consumptions
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_effects_immutable
    BEFORE UPDATE OR DELETE ON authorization_operator_effects
    FOR EACH ROW EXECUTE FUNCTION authorization_immutable_row_guard();
CREATE TRIGGER authorization_operator_effects_no_truncate
    BEFORE TRUNCATE ON authorization_operator_effects
    FOR EACH STATEMENT EXECUTE FUNCTION authorization_immutable_row_guard();
