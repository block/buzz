-- Crash-recoverable, privacy-bounded delivery for connection-local kind-24244
-- status. One authoritative transition may acquire a new target job after a
-- reconnect, but every dead target receives its own terminal outcome.

CREATE TABLE client_status_transition_heads (
    community_id UUID NOT NULL REFERENCES communities(id),
    subject_fingerprint BYTEA NOT NULL CHECK (octet_length(subject_fingerprint) = 32),
    signer_fingerprint BYTEA NOT NULL CHECK (octet_length(signer_fingerprint) = 32),
    transition_id UUID NOT NULL,
    status_revision BIGINT NOT NULL CHECK (status_revision > 0),
    delivery_kind SMALLINT NOT NULL CHECK (delivery_kind IN (1, 2)),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, subject_fingerprint, signer_fingerprint),
    UNIQUE (community_id, transition_id),
    CHECK (transition_id <> '00000000-0000-0000-0000-000000000000'::uuid)
);

CREATE TABLE client_status_transitions (
    community_id UUID NOT NULL REFERENCES communities(id),
    transition_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    delivery_kind SMALLINT NOT NULL CHECK (delivery_kind IN (1, 2)),
    subject_fingerprint BYTEA NOT NULL CHECK (octet_length(subject_fingerprint) = 32),
    signer_fingerprint BYTEA NOT NULL CHECK (octet_length(signer_fingerprint) = 32),
    status_revision BIGINT NOT NULL CHECK (status_revision > 0),
    supersedes_revision BIGINT CHECK (supersedes_revision > 0),
    signed_payload BYTEA NOT NULL CHECK (octet_length(signed_payload) BETWEEN 1 AND 4096),
    payload_digest BYTEA NOT NULL CHECK (
        octet_length(payload_digest) = 32
        AND payload_digest = digest(signed_payload, 'sha256')
    ),
    fresh_until TIMESTAMPTZ NOT NULL,
    allocated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    signed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    fenced_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    retain_until TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp() + INTERVAL '1 day',
    PRIMARY KEY (community_id, transition_id),
    UNIQUE (community_id, operation_id),
    UNIQUE (community_id, subject_fingerprint, signer_fingerprint, status_revision),
    UNIQUE (community_id, transition_id, subject_fingerprint, signer_fingerprint,
            status_revision, delivery_kind),
    CHECK (transition_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (request_fingerprint <> decode(repeat('00', 32), 'hex')),
    CHECK (subject_fingerprint <> decode(repeat('00', 32), 'hex')),
    CHECK (signer_fingerprint <> decode(repeat('00', 32), 'hex')),
    CHECK (
        (delivery_kind = 1 AND supersedes_revision IS NULL)
        OR (delivery_kind = 2 AND supersedes_revision IS NOT NULL
            AND status_revision > supersedes_revision)
    ),
    CHECK (signed_at >= allocated_at AND fenced_at >= signed_at),
    CHECK (fresh_until > fenced_at),
    CHECK (retain_until > fresh_until AND retain_until <= fresh_until + INTERVAL '1 day')
);

ALTER TABLE client_status_transition_heads
    ADD CONSTRAINT client_status_transition_heads_transition
    FOREIGN KEY (community_id, transition_id, subject_fingerprint, signer_fingerprint,
                 status_revision, delivery_kind)
    REFERENCES client_status_transitions
        (community_id, transition_id, subject_fingerprint, signer_fingerprint,
         status_revision, delivery_kind)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE client_status_delivery_outbox (
    community_id UUID NOT NULL REFERENCES communities(id),
    delivery_id UUID NOT NULL,
    transition_id UUID NOT NULL,
    connection_fingerprint BYTEA NOT NULL CHECK (
        octet_length(connection_fingerprint) = 32
        AND connection_fingerprint <> decode(repeat('00', 32), 'hex')
    ),
    delivery_state SMALLINT NOT NULL DEFAULT 1 CHECK (delivery_state IN (1, 2, 3)),
    attempt_count SMALLINT NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 16),
    claim_id UUID,
    completion_claim_id UUID,
    claimed_until TIMESTAMPTZ,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    last_failure_reason SMALLINT NOT NULL DEFAULT 0 CHECK (last_failure_reason BETWEEN 0 AND 7),
    created_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    attempt_started_at TIMESTAMPTZ,
    delivered_at TIMESTAMPTZ,
    terminal_at TIMESTAMPTZ,
    retain_until TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp() + INTERVAL '1 day',
    PRIMARY KEY (community_id, delivery_id),
    UNIQUE (community_id, transition_id, connection_fingerprint),
    FOREIGN KEY (community_id, transition_id)
        REFERENCES client_status_transitions (community_id, transition_id),
    CHECK (delivery_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK ((claim_id IS NULL) = (claimed_until IS NULL)),
    CHECK ((attempt_count = 0) = (attempt_started_at IS NULL)),
    CHECK (retain_until > created_at AND retain_until <= created_at + INTERVAL '1 day'),
    CHECK (
        (delivery_state = 1 AND delivered_at IS NULL AND terminal_at IS NULL
            AND completion_claim_id IS NULL)
        OR (delivery_state = 2 AND delivered_at IS NOT NULL AND terminal_at IS NULL
            AND last_failure_reason = 0 AND claim_id IS NULL
            AND completion_claim_id IS NOT NULL)
        OR (delivery_state = 3 AND delivered_at IS NULL AND terminal_at IS NOT NULL
            AND last_failure_reason BETWEEN 2 AND 7 AND claim_id IS NULL
            AND completion_claim_id IS NOT NULL)
    )
);

CREATE TABLE client_status_delivery_capacity (
    community_id UUID NOT NULL REFERENCES communities(id),
    pending_count INTEGER NOT NULL DEFAULT 0 CHECK (pending_count BETWEEN 0 AND 1024),
    total_count INTEGER NOT NULL DEFAULT 0 CHECK (total_count BETWEEN 0 AND 8192),
    healthy BOOLEAN NOT NULL DEFAULT TRUE,
    failure_reason SMALLINT NOT NULL DEFAULT 0 CHECK (failure_reason BETWEEN 0 AND 3),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    CHECK ((healthy AND failure_reason = 0) OR (NOT healthy AND failure_reason BETWEEN 1 AND 3)),
    CHECK (pending_count <= total_count),
    PRIMARY KEY (community_id)
);

CREATE INDEX client_status_delivery_outbox_ready
    ON client_status_delivery_outbox
        (community_id, connection_fingerprint, next_attempt_at, created_at, delivery_id)
    WHERE delivery_state = 1;

CREATE INDEX client_status_delivery_outbox_retention
    ON client_status_delivery_outbox (community_id, retain_until, delivery_id)
    WHERE delivery_state IN (2, 3);

CREATE TABLE client_status_delivery_events (
    community_id UUID NOT NULL,
    delivery_id UUID NOT NULL,
    event_sequence SMALLINT NOT NULL CHECK (event_sequence BETWEEN 1 AND 64),
    event_kind SMALLINT NOT NULL CHECK (event_kind BETWEEN 1 AND 8),
    reason_code SMALLINT NOT NULL CHECK (reason_code BETWEEN 0 AND 7),
    attempt_count SMALLINT NOT NULL CHECK (attempt_count BETWEEN 0 AND 16),
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, delivery_id, event_sequence),
    FOREIGN KEY (community_id, delivery_id)
        REFERENCES client_status_delivery_outbox (community_id, delivery_id)
        ON DELETE CASCADE,
    CHECK ((event_kind BETWEEN 1 AND 5 AND reason_code = 0)
        OR (event_kind BETWEEN 6 AND 8 AND reason_code BETWEEN 1 AND 7))
);

CREATE FUNCTION client_status_delivery_capacity_guard_v1() RETURNS TRIGGER AS $$
DECLARE capacity client_status_delivery_capacity%ROWTYPE;
BEGIN
    SELECT * INTO capacity FROM client_status_delivery_capacity
     WHERE community_id = NEW.community_id FOR UPDATE;
    IF NOT FOUND OR NOT capacity.healthy THEN
        RAISE EXCEPTION 'client status delivery audit is unavailable'
            USING ERRCODE = 'object_not_in_prerequisite_state',
                  CONSTRAINT = 'client_status_delivery_capacity_health';
    END IF;
    IF capacity.pending_count >= 1024 OR capacity.total_count >= 8192 THEN
        RAISE EXCEPTION 'client status delivery capacity exhausted'
            USING ERRCODE = 'program_limit_exceeded',
                  CONSTRAINT = 'client_status_delivery_capacity';
    END IF;
    NEW.created_at := transaction_timestamp();
    NEW.next_attempt_at := transaction_timestamp();
    NEW.retain_until := transaction_timestamp() + INTERVAL '1 day';
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER client_status_delivery_capacity
    BEFORE INSERT ON client_status_delivery_outbox
    FOR EACH ROW EXECUTE FUNCTION client_status_delivery_capacity_guard_v1();

CREATE FUNCTION client_status_delivery_capacity_insert_v1() RETURNS TRIGGER AS $$
BEGIN
    UPDATE client_status_delivery_capacity SET
        pending_count = pending_count + 1,
        total_count = total_count + 1,
        updated_at = transaction_timestamp()
     WHERE community_id = NEW.community_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'client status delivery capacity row is missing'
            USING ERRCODE = 'object_not_in_prerequisite_state',
                  CONSTRAINT = 'client_status_delivery_capacity_health';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER client_status_delivery_capacity_insert
    AFTER INSERT ON client_status_delivery_outbox
    FOR EACH ROW EXECUTE FUNCTION client_status_delivery_capacity_insert_v1();

CREATE FUNCTION client_status_delivery_capacity_state_v1() RETURNS TRIGGER AS $$
BEGIN
    IF OLD.delivery_state = 1 AND NEW.delivery_state IN (2, 3) THEN
        UPDATE client_status_delivery_capacity SET
            pending_count = pending_count - 1,
            updated_at = transaction_timestamp()
         WHERE community_id = NEW.community_id;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'client status delivery capacity row is missing'
                USING ERRCODE = 'object_not_in_prerequisite_state',
                      CONSTRAINT = 'client_status_delivery_capacity_health';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER client_status_delivery_capacity_state
    AFTER UPDATE ON client_status_delivery_outbox
    FOR EACH ROW EXECUTE FUNCTION client_status_delivery_capacity_state_v1();

CREATE FUNCTION client_status_delivery_capacity_delete_v1() RETURNS TRIGGER AS $$
BEGIN
    UPDATE client_status_delivery_capacity SET
        total_count = total_count - 1,
        updated_at = transaction_timestamp()
     WHERE community_id = OLD.community_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'client status delivery capacity row is missing'
            USING ERRCODE = 'object_not_in_prerequisite_state',
                  CONSTRAINT = 'client_status_delivery_capacity_health';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER client_status_delivery_capacity_delete
    AFTER DELETE ON client_status_delivery_outbox
    FOR EACH ROW EXECUTE FUNCTION client_status_delivery_capacity_delete_v1();

CREATE FUNCTION client_status_transition_immutable_v1() RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'client status transition is immutable'
        USING ERRCODE = 'check_violation', CONSTRAINT = 'client_status_transition_immutable';
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER client_status_transition_no_update
    BEFORE UPDATE ON client_status_transitions
    FOR EACH ROW EXECUTE FUNCTION client_status_transition_immutable_v1();
CREATE FUNCTION client_status_transition_head_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.status_revision <> 1 THEN
            RAISE EXCEPTION 'client status head must start at revision one'
                USING ERRCODE = 'check_violation', CONSTRAINT = 'client_status_transition_head_revision';
        END IF;
    ELSIF NEW.community_id IS DISTINCT FROM OLD.community_id
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
CREATE TRIGGER client_status_transition_head_state
    BEFORE INSERT OR UPDATE ON client_status_transition_heads
    FOR EACH ROW EXECUTE FUNCTION client_status_transition_head_guard_v1();
CREATE FUNCTION client_status_transition_retain_v1() RETURNS TRIGGER AS $$
BEGIN
    IF transaction_timestamp() <= OLD.retain_until THEN
        RAISE EXCEPTION 'client status transition is still retained'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'client_status_transition_retention';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER client_status_transition_retention
    BEFORE DELETE ON client_status_transitions
    FOR EACH ROW EXECUTE FUNCTION client_status_transition_retain_v1();

CREATE FUNCTION client_status_delivery_state_guard_v1() RETURNS TRIGGER AS $$
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
         AND head.subject_fingerprint=transition.subject_fingerprint
         AND head.signer_fingerprint=transition.signer_fingerprint
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

CREATE TRIGGER client_status_delivery_state
    BEFORE UPDATE ON client_status_delivery_outbox
    FOR EACH ROW EXECUTE FUNCTION client_status_delivery_state_guard_v1();

CREATE FUNCTION client_status_delivery_retain_v1() RETURNS TRIGGER AS $$
BEGIN
    IF OLD.delivery_state = 1 OR transaction_timestamp() <= OLD.retain_until THEN
        RAISE EXCEPTION 'client status delivery is still retained'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'client_status_delivery_retention';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER client_status_delivery_retention
    BEFORE DELETE ON client_status_delivery_outbox
    FOR EACH ROW EXECUTE FUNCTION client_status_delivery_retain_v1();

CREATE FUNCTION client_status_delivery_event_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'client status delivery event is immutable'
        USING ERRCODE = 'check_violation', CONSTRAINT = 'client_status_delivery_event_immutable';
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER client_status_delivery_event_no_update
    BEFORE UPDATE ON client_status_delivery_events
    FOR EACH ROW EXECUTE FUNCTION client_status_delivery_event_guard_v1();
CREATE FUNCTION client_status_delivery_event_retain_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM client_status_delivery_outbox delivery
        WHERE delivery.community_id = OLD.community_id
          AND delivery.delivery_id = OLD.delivery_id
          AND delivery.delivery_state IN (2, 3)
          AND transaction_timestamp() > delivery.retain_until
    ) THEN
        RAISE EXCEPTION 'client status delivery event is still retained'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'client_status_delivery_event_retention';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER client_status_delivery_event_retention
    BEFORE DELETE ON client_status_delivery_events
    FOR EACH ROW EXECUTE FUNCTION client_status_delivery_event_retain_v1();
CREATE TRIGGER client_status_delivery_event_no_truncate
    BEFORE TRUNCATE ON client_status_delivery_events
    FOR EACH STATEMENT EXECUTE FUNCTION client_status_delivery_event_guard_v1();
CREATE TRIGGER client_status_transition_no_truncate
    BEFORE TRUNCATE ON client_status_transitions
    FOR EACH STATEMENT EXECUTE FUNCTION client_status_delivery_event_guard_v1();
CREATE TRIGGER client_status_delivery_no_truncate
    BEFORE TRUNCATE ON client_status_delivery_outbox
    FOR EACH STATEMENT EXECUTE FUNCTION client_status_delivery_event_guard_v1();
