-- Reconcile the crash-durable journal with the accepted S5 rule that status
-- revisions and withdrawals are scoped to one exact connection generation.
-- The relay producer did not exist before this migration, so any transition
-- row would be evidence of an unsupported writer and must stop rollout.

LOCK TABLE client_status_transitions, client_status_transition_heads,
    client_status_delivery_outbox IN ACCESS EXCLUSIVE MODE;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM client_status_transitions) THEN
        RAISE EXCEPTION 'client status connection-scope migration requires an empty journal'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END;
$$;

-- A status authorization holds FOR SHARE on the same community row until the
-- physical writer acknowledgement is durably completed. Every newer policy
-- revision therefore waits only for status flushes in its own tenant.
CREATE OR REPLACE FUNCTION client_status_policy_connection_fence_v1() RETURNS TRIGGER AS $$
BEGIN
    PERFORM id FROM communities WHERE id=NEW.community_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'client status policy community is unavailable'
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER client_status_policy_connection_fence
    BEFORE INSERT ON identity_enrollment_policies
    FOR EACH ROW EXECUTE FUNCTION client_status_policy_connection_fence_v1();

ALTER TABLE client_status_transitions
    ADD COLUMN connection_fingerprint BYTEA NOT NULL CHECK (
        octet_length(connection_fingerprint) = 32
        AND connection_fingerprint <> decode(repeat('00', 32), 'hex')
    ),
    ADD COLUMN evidence_author_pubkey BYTEA CHECK (
        evidence_author_pubkey IS NULL OR octet_length(evidence_author_pubkey) = 32
    ),
    ADD COLUMN evidence_binding_id UUID,
    ADD COLUMN evidence_binding_version BIGINT CHECK (
        evidence_binding_version IS NULL OR evidence_binding_version > 0
    ),
    ADD COLUMN evidence_policy_revision BIGINT CHECK (
        evidence_policy_revision IS NULL OR evidence_policy_revision > 0
    ),
    ADD COLUMN evidence_invalidation_generation BIGINT CHECK (
        evidence_invalidation_generation IS NULL OR evidence_invalidation_generation >= 0
    ),
    ADD COLUMN evidence_authority_epoch BIGINT CHECK (
        evidence_authority_epoch IS NULL OR evidence_authority_epoch > 0
    ),
    ADD COLUMN evidence_fence BYTEA CHECK (
        evidence_fence IS NULL
        OR (octet_length(evidence_fence) = 32
            AND evidence_fence <> decode(repeat('00', 32), 'hex'))
    ),
    ADD COLUMN evidence_observed_at TIMESTAMPTZ,
    ADD CONSTRAINT client_status_transition_private_evidence CHECK (
        (delivery_kind = 1
            AND evidence_author_pubkey IS NOT NULL
            AND evidence_binding_id IS NOT NULL
            AND evidence_binding_id <> '00000000-0000-0000-0000-000000000000'::uuid
            AND evidence_binding_version IS NOT NULL
            AND evidence_policy_revision IS NOT NULL
            AND evidence_invalidation_generation IS NOT NULL
            AND evidence_authority_epoch IS NOT NULL
            AND evidence_fence IS NOT NULL
            AND evidence_observed_at IS NOT NULL
            AND evidence_observed_at < fresh_until)
        OR (delivery_kind = 2
            AND evidence_author_pubkey IS NULL
            AND evidence_binding_id IS NULL
            AND evidence_binding_version IS NULL
            AND evidence_policy_revision IS NULL
            AND evidence_invalidation_generation IS NULL
            AND evidence_authority_epoch IS NULL
            AND evidence_fence IS NULL
            AND evidence_observed_at IS NULL)
    );

ALTER TABLE client_status_transition_heads
    ADD COLUMN connection_fingerprint BYTEA NOT NULL CHECK (
        octet_length(connection_fingerprint) = 32
        AND connection_fingerprint <> decode(repeat('00', 32), 'hex')
    );

ALTER TABLE client_status_transition_heads
    DROP CONSTRAINT client_status_transition_heads_transition,
    DROP CONSTRAINT client_status_transition_heads_pkey;

DO $$
DECLARE constraint_name TEXT;
BEGIN
    SELECT c.conname INTO constraint_name
      FROM pg_constraint c
     WHERE c.conrelid = 'client_status_transition_heads'::regclass
       AND c.contype = 'u'
       AND pg_get_constraintdef(c.oid)
           = 'UNIQUE (community_id, transition_id)';
    IF constraint_name IS NOT NULL THEN
        EXECUTE format(
            'ALTER TABLE client_status_transition_heads DROP CONSTRAINT %I',
            constraint_name
        );
    END IF;

    SELECT c.conname INTO constraint_name
      FROM pg_constraint c
     WHERE c.conrelid = 'client_status_transitions'::regclass
       AND c.contype = 'u'
       AND pg_get_constraintdef(c.oid)
           = 'UNIQUE (community_id, subject_fingerprint, signer_fingerprint, status_revision)';
    IF constraint_name IS NOT NULL THEN
        EXECUTE format(
            'ALTER TABLE client_status_transitions DROP CONSTRAINT %I',
            constraint_name
        );
    END IF;

    SELECT c.conname INTO constraint_name
      FROM pg_constraint c
     WHERE c.conrelid = 'client_status_transitions'::regclass
       AND c.contype = 'u'
       AND pg_get_constraintdef(c.oid)
           = 'UNIQUE (community_id, transition_id, subject_fingerprint, signer_fingerprint, status_revision, delivery_kind)';
    IF constraint_name IS NOT NULL THEN
        EXECUTE format(
            'ALTER TABLE client_status_transitions DROP CONSTRAINT %I',
            constraint_name
        );
    END IF;
END;
$$;

ALTER TABLE client_status_transitions
    ADD CONSTRAINT client_status_transition_connection_revision
        UNIQUE (community_id, connection_fingerprint, status_revision),
    ADD CONSTRAINT client_status_transition_connection_identity
        UNIQUE (community_id, transition_id, connection_fingerprint,
                subject_fingerprint, signer_fingerprint, status_revision, delivery_kind);

ALTER TABLE client_status_transition_heads
    ADD PRIMARY KEY (community_id, connection_fingerprint),
    ADD UNIQUE (community_id, transition_id, connection_fingerprint),
    ADD CONSTRAINT client_status_transition_heads_transition
        FOREIGN KEY (community_id, transition_id, connection_fingerprint,
                     subject_fingerprint, signer_fingerprint, status_revision, delivery_kind)
        REFERENCES client_status_transitions
            (community_id, transition_id, connection_fingerprint,
             subject_fingerprint, signer_fingerprint, status_revision, delivery_kind)
        DEFERRABLE INITIALLY DEFERRED;

CREATE OR REPLACE FUNCTION client_status_transition_head_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.status_revision <> 1 THEN
            RAISE EXCEPTION 'client status head must start at revision one'
                USING ERRCODE = 'check_violation', CONSTRAINT = 'client_status_transition_head_revision';
        END IF;
    ELSIF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.connection_fingerprint IS DISTINCT FROM OLD.connection_fingerprint
        OR NEW.subject_fingerprint IS DISTINCT FROM OLD.subject_fingerprint
        OR NEW.signer_fingerprint IS DISTINCT FROM OLD.signer_fingerprint
        OR NEW.status_revision <> OLD.status_revision + 1
        OR NEW.transition_id IS NOT DISTINCT FROM OLD.transition_id
    THEN
        RAISE EXCEPTION 'client status head transition is invalid'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'client_status_transition_head_revision';
    END IF;
    NEW.updated_at := transaction_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION client_status_delivery_state_guard_v1() RETURNS TRIGGER AS $$
DECLARE claim_advance BOOLEAN; retry_advance BOOLEAN; delivered_advance BOOLEAN; terminal_advance BOOLEAN;
DECLARE expired_transition BOOLEAN; stale_transition BOOLEAN;
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.delivery_id IS DISTINCT FROM OLD.delivery_id
        OR NEW.transition_id IS DISTINCT FROM OLD.transition_id
        OR NEW.connection_fingerprint IS DISTINCT FROM OLD.connection_fingerprint
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
        OR NEW.retain_until IS DISTINCT FROM OLD.retain_until
        OR OLD.delivery_state <> 1
    THEN
        RAISE EXCEPTION 'client status delivery transition is invalid'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'client_status_delivery_transition';
    END IF;
    SELECT EXISTS(SELECT 1 FROM client_status_transitions transition
        WHERE transition.community_id=OLD.community_id
          AND transition.transition_id=OLD.transition_id
          AND transition.fresh_until <= transaction_timestamp())
      INTO expired_transition;
    SELECT NOT EXISTS(
        SELECT 1 FROM client_status_delivery_outbox delivery
        JOIN client_status_transitions transition
          ON transition.community_id=delivery.community_id
         AND transition.transition_id=delivery.transition_id
        JOIN client_status_transition_heads head
          ON head.community_id=transition.community_id
         AND head.connection_fingerprint=delivery.connection_fingerprint
         AND head.transition_id=transition.transition_id
        WHERE delivery.community_id=OLD.community_id
          AND delivery.delivery_id=OLD.delivery_id)
      INTO stale_transition;
    claim_advance := NEW.delivery_state = 1 AND NEW.claim_id IS NOT NULL
        AND NEW.completion_claim_id IS NULL AND NEW.attempt_count = OLD.attempt_count + 1
        AND NOT stale_transition AND NOT expired_transition
        AND OLD.next_attempt_at <= transaction_timestamp() AND OLD.attempt_count < 16
        AND (OLD.claim_id IS NULL OR OLD.claimed_until <= transaction_timestamp())
        AND NEW.claim_id IS DISTINCT FROM OLD.claim_id
        AND NEW.claimed_until > transaction_timestamp()
        AND NEW.claimed_until <= transaction_timestamp() + INTERVAL '5 minutes'
        AND NEW.attempt_started_at IS NOT NULL
        AND (NEW.last_failure_reason = OLD.last_failure_reason
            OR (OLD.claim_id IS NOT NULL AND OLD.claimed_until <= transaction_timestamp()
                AND NEW.last_failure_reason = 7))
        AND NEW.next_attempt_at = OLD.next_attempt_at;
    retry_advance := NEW.delivery_state = 1 AND OLD.claim_id IS NOT NULL AND NEW.claim_id IS NULL
        AND NEW.completion_claim_id IS NULL AND NEW.attempt_count = OLD.attempt_count
        AND NEW.attempt_started_at = OLD.attempt_started_at AND NEW.last_failure_reason = 1
        AND NEW.next_attempt_at > OLD.next_attempt_at;
    delivered_advance := NEW.delivery_state = 2 AND OLD.claim_id IS NOT NULL
        AND NEW.claim_id IS NULL AND NEW.completion_claim_id = OLD.claim_id
        AND NEW.attempt_count = OLD.attempt_count AND NEW.delivered_at IS NOT NULL
        AND NEW.terminal_at IS NULL AND NEW.last_failure_reason = 0;
    terminal_advance := NEW.delivery_state = 3 AND NEW.claim_id IS NULL
        AND NEW.completion_claim_id IS NOT NULL
        AND NEW.attempt_count = OLD.attempt_count AND NEW.delivered_at IS NULL
        AND NEW.terminal_at IS NOT NULL
        AND (
            (OLD.claim_id IS NOT NULL AND NEW.completion_claim_id = OLD.claim_id
                AND NEW.last_failure_reason BETWEEN 2 AND 7)
            OR (NEW.last_failure_reason = 2
                AND (OLD.claim_id IS NULL OR NEW.completion_claim_id = OLD.claim_id))
            OR (NEW.last_failure_reason = 3 AND stale_transition
                AND (OLD.claim_id IS NULL OR OLD.claimed_until <= transaction_timestamp()))
            OR (NEW.last_failure_reason = 5 AND expired_transition
                AND (OLD.claim_id IS NULL OR OLD.claimed_until <= transaction_timestamp()))
            OR (NEW.last_failure_reason = 6 AND OLD.attempt_count >= 16
                AND (OLD.claim_id IS NULL OR OLD.claimed_until <= transaction_timestamp()))
        );
    IF NOT (claim_advance OR retry_advance OR delivered_advance OR terminal_advance) THEN
        RAISE EXCEPTION 'client status delivery transition is invalid'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'client_status_delivery_transition';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
