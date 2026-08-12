-- NIP-FI identity lifecycle transaction closure.
--
-- Migration 0029 already owns the direct-final binding, lifecycle, selector,
-- consumption, and receipt constraints. Migration 0030 owns immutable bounded
-- authorization events. This migration closes their one missing cross-table
-- invariant without recreating or weakening either foundation.

-- Serialize an exact operation before checking the sole receipt row. Identity
-- principal/key coordinate locks remain separate and retain v29's canonical
-- numeric ordering.
CREATE FUNCTION nip_fi_lock_authorization_operation_v1(
    locked_community_id UUID,
    locked_operation_id UUID
) RETURNS VOID AS $$
BEGIN
    IF locked_community_id = '00000000-0000-0000-0000-000000000000'::uuid
        OR locked_operation_id = '00000000-0000-0000-0000-000000000000'::uuid
    THEN
        RAISE EXCEPTION 'invalid NIP-FI operation lock coordinate'
            USING ERRCODE = 'check_violation';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'buzz:authorization-operation:v1:' || locked_community_id::text
            || ':' || locked_operation_id::text,
        0
    ));
END;
$$ LANGUAGE plpgsql VOLATILE STRICT PARALLEL UNSAFE;

-- Canonical admission keeps its complete logical intent and the closed,
-- credential-free application result beside the immutable receipt. This is
-- what lets an identical request replay reconstruct the same typed result
-- without repeating membership or other application DML.
CREATE TABLE authorization_admission_results (
    community_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    semantic_fingerprint BYTEA NOT NULL CHECK (
        octet_length(semantic_fingerprint) = 32
        AND semantic_fingerprint <> decode(repeat('00', 32), 'hex')
    ),
    object_kind SMALLINT NOT NULL CHECK (object_kind BETWEEN 1 AND 6),
    object_key BYTEA NOT NULL CHECK (
        octet_length(object_key) = 32
        AND object_key <> decode(repeat('00', 32), 'hex')
    ),
    application_type BYTEA CHECK (
        application_type IS NULL
        OR (octet_length(application_type) = 32
            AND application_type <> decode(repeat('00', 32), 'hex'))
    ),
    application_version SMALLINT CHECK (application_version > 0),
    application_code SMALLINT CHECK (application_code > 0),
    application_payload BYTEA CHECK (
        application_payload IS NULL OR octet_length(application_payload) <= 4096
    ),
    application_intent_digest BYTEA CHECK (
        application_intent_digest IS NULL
        OR (octet_length(application_intent_digest) = 32
            AND application_intent_digest <> decode(repeat('00', 32), 'hex'))
    ),
    application_effect_digest BYTEA CHECK (
        application_effect_digest IS NULL
        OR (octet_length(application_effect_digest) = 32
            AND application_effect_digest <> decode(repeat('00', 32), 'hex'))
    ),
    application_result_digest BYTEA CHECK (
        application_result_digest IS NULL
        OR (octet_length(application_result_digest) = 32
            AND application_result_digest <> decode(repeat('00', 32), 'hex'))
    ),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, operation_id),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint),
    CHECK (
        (application_type IS NULL
            AND application_version IS NULL
            AND application_code IS NULL
            AND application_payload IS NULL
            AND application_intent_digest IS NULL
            AND application_effect_digest IS NULL
            AND application_result_digest IS NULL)
        OR (application_type IS NOT NULL
            AND application_version IS NOT NULL
            AND application_code IS NOT NULL
            AND application_payload IS NOT NULL
            AND application_intent_digest IS NOT NULL
            AND application_effect_digest IS NOT NULL
            AND application_result_digest IS NOT NULL)
    )
);

CREATE TRIGGER authorization_admission_results_no_update
    BEFORE UPDATE OR DELETE ON authorization_admission_results
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_admission_results_no_truncate
    BEFORE TRUNCATE ON authorization_admission_results
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

-- Every successful/no-op lifecycle receipt has exactly one privacy-safe audit
-- event with the closed transition-kind mapping. Both directions are deferred
-- so receipt, history, event, selectors, binding, and consumption may be
-- inserted in any order inside one transaction but can never commit partially.
CREATE FUNCTION authorization_operation_receipt_event_guard_v1()
RETURNS TRIGGER AS $$
DECLARE
    receipt authorization_operation_receipts%ROWTYPE;
    expected_event_kind SMALLINT;
    matching_event_count BIGINT;
    expected_event_count BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'authorization_operation_receipts' THEN
        receipt := NEW;
    ELSE
        SELECT * INTO receipt
        FROM authorization_operation_receipts
        WHERE community_id = NEW.community_id
          AND operation_id = NEW.operation_id;
        IF NOT FOUND THEN
            -- Credential-free pre-authentication denials intentionally have no
            -- canonical receipt. Their separate v30 FK/shape guards still run.
            RETURN NULL;
        END IF;
    END IF;

    IF receipt.operation_kind NOT BETWEEN 1 AND 9 THEN
        RETURN NULL;
    END IF;

    expected_event_kind := CASE receipt.operation_kind
        WHEN 1 THEN 1  -- enroll
        WHEN 2 THEN 1  -- provision
        WHEN 3 THEN 6  -- retire
        WHEN 4 THEN 7  -- disable
        WHEN 5 THEN 2  -- revoke
        WHEN 6 THEN 3  -- rotate
        WHEN 7 THEN 4  -- recover
        WHEN 8 THEN 5  -- enable
        WHEN 9 THEN 8  -- admission loss
    END;

    SELECT
        count(*),
        count(*) FILTER (WHERE event_kind = expected_event_kind)
    INTO matching_event_count, expected_event_count
    FROM authorization_events
    WHERE community_id = receipt.community_id
      AND operation_id = receipt.operation_id
      AND request_fingerprint = receipt.request_fingerprint;

    IF matching_event_count <> 1 OR expected_event_count <> 1 THEN
        RAISE EXCEPTION
            'lifecycle receipt requires exactly one event kind %, found % total and % expected',
            expected_event_kind, matching_event_count, expected_event_count
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_operation_receipt_event_cardinality';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER authorization_operation_receipt_event_cardinality
    AFTER INSERT ON authorization_operation_receipts
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operation_receipt_event_guard_v1();

CREATE CONSTRAINT TRIGGER authorization_event_receipt_cardinality
    AFTER INSERT ON authorization_events
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operation_receipt_event_guard_v1();
