-- NIP-FI trusted-proxy replay claims and protected publication projections.
--
-- Migration 0030 already owns protected-object authority and invalidation
-- generations. This migration does not duplicate that authority, lifecycle,
-- admission, audit, or audio-lease state. Audio leases remain memory-only and
-- die on restart.

-- Credential-free replay claims consumed only by canonical final admission.
-- claim_kind: 1 trusted-proxy nonce. The key is a domain-separated SHA-256
-- digest of the decoded nonce; the server-owned authorization domain is the
-- first component of the frozen uniqueness key. Raw nonce, assertion,
-- provider, identity, and credential material are forbidden here.
CREATE TABLE authorization_proxy_nonce_claims (
    authorization_domain UUID NOT NULL REFERENCES communities(id),
    claim_kind SMALLINT NOT NULL CHECK (claim_kind = 1),
    claim_key BYTEA NOT NULL CHECK (
        octet_length(claim_key) = 32
        AND claim_key <> decode(repeat('00', 32), 'hex')
    ),
    committed_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    retain_until TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (authorization_domain, claim_kind, claim_key),
    CHECK (committed_at < retain_until)
);

-- Replay claims are deployment admission control rather than tenant-visible
-- application data. The authorization domain remains the first component of
-- every key and prevents cross-domain substitution.
INSERT INTO _operator_global_tables (table_name, reason) VALUES (
    'authorization_proxy_nonce_claims',
    'server replay ledger; authorization_domain is the isolation key'
);

CREATE FUNCTION authorization_proxy_nonce_claim_stamp_v1() RETURNS TRIGGER AS $$
BEGIN
    NEW.committed_at := transaction_timestamp();
    IF NEW.retain_until <= NEW.committed_at THEN
        RAISE EXCEPTION 'trusted-proxy nonce claim is already expired'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_proxy_nonce_claim_expired';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_proxy_nonce_claims_stamp
    BEFORE INSERT ON authorization_proxy_nonce_claims
    FOR EACH ROW EXECUTE FUNCTION authorization_proxy_nonce_claim_stamp_v1();

-- Claims may be reclaimed only after their exclusive retention bound. UPDATE
-- cannot change a committed claim and TRUNCATE cannot bypass row retention.
CREATE FUNCTION authorization_proxy_nonce_claim_retention_v1() RETURNS TRIGGER AS $$
BEGIN
    IF transaction_timestamp() <= OLD.retain_until THEN
        RAISE EXCEPTION 'trusted-proxy nonce claim retention is still active'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_proxy_nonce_claim_retention';
    END IF;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_proxy_nonce_claims_immutable
    BEFORE UPDATE ON authorization_proxy_nonce_claims
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_proxy_nonce_claims_retained
    BEFORE DELETE ON authorization_proxy_nonce_claims
    FOR EACH ROW EXECUTE FUNCTION authorization_proxy_nonce_claim_retention_v1();
CREATE TRIGGER authorization_proxy_nonce_claims_no_truncate
    BEFORE TRUNCATE ON authorization_proxy_nonce_claims
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

-- Durable post-commit projection work. Immutable staged objects are safe when
-- abandoned; a canonical applied publication atomically inserts these rows so
-- a worker can deterministically resume cache/moderation projection after a
-- process or dependency failure. Projection kinds: 1 media moderation record,
-- 2 media sidecar cache, 3 Git pointer cache.
--
-- delivery_state: 1 pending, 2 delivered, 3 terminal.
-- failure_code: 0 none; 1 transient object-store failure; 2 dependency
-- unavailable; 3 existing bytes conflict; 4 staged object missing/corrupt;
-- 5 publication/receipt digest mismatch; 6 tenant/target mismatch.
CREATE SEQUENCE protected_publication_projection_sequence_v1 AS BIGINT
    START WITH 1 INCREMENT BY 1 NO MINVALUE NO MAXVALUE CACHE 1 NO CYCLE;

CREATE TABLE protected_publication_projection_outbox (
    community_id UUID NOT NULL REFERENCES communities(id),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (
        octet_length(request_fingerprint) = 32
        AND request_fingerprint <> decode(repeat('00', 32), 'hex')
    ),
    publication_result_digest BYTEA NOT NULL CHECK (
        octet_length(publication_result_digest) = 32
        AND publication_result_digest <> decode(repeat('00', 32), 'hex')
    ),
    object_kind SMALLINT NOT NULL CHECK (object_kind IN (3, 4)),
    object_key BYTEA NOT NULL CHECK (
        octet_length(object_key) = 32
        AND object_key <> decode(repeat('00', 32), 'hex')
    ),
    application_type BYTEA NOT NULL CHECK (
        octet_length(application_type) = 32
        AND application_type <> decode(repeat('00', 32), 'hex')
    ),
    application_version SMALLINT NOT NULL CHECK (application_version > 0),
    application_effect_digest BYTEA NOT NULL CHECK (
        octet_length(application_effect_digest) = 32
        AND application_effect_digest <> decode(repeat('00', 32), 'hex')
    ),
    -- Filled by the deferred receipt guard after canonical authority has been
    -- persisted. The publication operation may be a same-epoch successor, so
    -- these fields intentionally identify the authority origin separately.
    authority_operation_id UUID,
    authority_request_fingerprint BYTEA,
    authority_epoch BIGINT,
    authority_fence BYTEA,
    projection_kind SMALLINT NOT NULL CHECK (projection_kind IN (1, 2, 3)),
    projection_key TEXT COLLATE "C" NOT NULL CHECK (
        octet_length(projection_key) BETWEEN 1 AND 2048
    ),
    staged_object_key TEXT COLLATE "C" NOT NULL CHECK (
        octet_length(staged_object_key) BETWEEN 1 AND 2048
    ),
    payload_digest BYTEA NOT NULL CHECK (
        octet_length(payload_digest) = 32
        AND payload_digest <> decode(repeat('00', 32), 'hex')
    ),
    delivery_state SMALLINT NOT NULL DEFAULT 1 CHECK (delivery_state IN (1, 2, 3)),
    attempt_count BIGINT NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    last_attempt_at TIMESTAMPTZ,
    delivered_at TIMESTAMPTZ,
    failure_code SMALLINT NOT NULL DEFAULT 0 CHECK (failure_code BETWEEN 0 AND 6),
    created_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    publication_sequence BIGINT NOT NULL,
    PRIMARY KEY (community_id, operation_id, projection_kind),
    UNIQUE (community_id, publication_sequence),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT protected_publication_projection_authority_binding CHECK (
        (authority_operation_id IS NULL
            AND authority_request_fingerprint IS NULL
            AND authority_epoch IS NULL
            AND authority_fence IS NULL)
        OR (authority_operation_id IS NOT NULL
            AND authority_operation_id <> '00000000-0000-0000-0000-000000000000'::uuid
            AND authority_request_fingerprint IS NOT NULL
            AND octet_length(authority_request_fingerprint) = 32
            AND authority_epoch IS NOT NULL
            AND authority_epoch > 0
            AND authority_fence IS NOT NULL
            AND octet_length(authority_fence) = 32
            AND authority_fence <> decode(repeat('00', 32), 'hex'))
    ),
    CHECK ((attempt_count = 0) = (last_attempt_at IS NULL)),
    CHECK (last_attempt_at IS NULL OR last_attempt_at >= created_at),
    CHECK (delivered_at IS NULL OR delivered_at >= created_at),
    CHECK (
        (delivery_state = 1 AND delivered_at IS NULL AND failure_code IN (0, 1, 2))
        OR (delivery_state = 2 AND delivered_at IS NOT NULL AND failure_code = 0)
        OR (delivery_state = 3 AND delivered_at IS NULL AND failure_code BETWEEN 3 AND 6)
    )
);

ALTER SEQUENCE protected_publication_projection_sequence_v1
    OWNED BY protected_publication_projection_outbox.publication_sequence;

CREATE UNIQUE INDEX protected_publication_projection_outbox_active_target
    ON protected_publication_projection_outbox
        (community_id, projection_kind, projection_key)
    WHERE delivery_state = 1;

CREATE INDEX protected_publication_projection_outbox_pending
    ON protected_publication_projection_outbox
        (delivery_state, next_attempt_at, community_id, operation_id)
    WHERE delivery_state = 1;

CREATE INDEX protected_publication_projection_outbox_target_order
    ON protected_publication_projection_outbox
        (community_id, object_kind, object_key, projection_kind,
         projection_key, publication_sequence);

-- Every projection starts immediately pending. Delivery timestamps and retry
-- counters are database-owned so a caller cannot manufacture a completed
-- projection or suppress the first durable attempt.
CREATE FUNCTION protected_publication_projection_insert_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.delivery_state <> 1
        OR NEW.attempt_count <> 0
        OR NEW.last_attempt_at IS NOT NULL
        OR NEW.delivered_at IS NOT NULL
        OR NEW.failure_code <> 0
        OR NEW.authority_operation_id IS NOT NULL
        OR NEW.authority_request_fingerprint IS NOT NULL
        OR NEW.authority_epoch IS NOT NULL
        OR NEW.authority_fence IS NOT NULL
    THEN
        RAISE EXCEPTION 'protected publication projection must start pending'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'protected_publication_projection_initial_state';
    END IF;
    -- Serialize only publications for the exact protected projection target.
    -- Sequence allocation happens after acquiring this transaction lock, so a
    -- later publication cannot become visible or deliver before an earlier
    -- allocator commits or aborts. Hash collisions only over-serialize two
    -- independent targets; they cannot weaken ordering.
    PERFORM pg_advisory_xact_lock(
        hashtextextended(
            jsonb_build_array(
                'buzz:nip-fi:protected-publication-projection:v1',
                NEW.community_id::TEXT,
                NEW.projection_kind,
                NEW.projection_key
            )::TEXT,
            0
        )
    );
    NEW.publication_sequence := nextval('protected_publication_projection_sequence_v1');
    NEW.created_at := transaction_timestamp();
    NEW.next_attempt_at := transaction_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER protected_publication_projection_initial_state
    BEFORE INSERT ON protected_publication_projection_outbox
    FOR EACH ROW EXECUTE FUNCTION protected_publication_projection_insert_guard_v1();

-- The outbox cannot be attached to a denial, no-op, lifecycle operation, or a
-- different result. The deferred check permits receipt and outbox insertion in
-- either order inside the canonical application transaction.
CREATE FUNCTION protected_publication_projection_receipt_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    canonical_authority_operation_id UUID;
    canonical_authority_request_fingerprint BYTEA;
    canonical_authority_epoch BIGINT;
    canonical_authority_fence BYTEA;
BEGIN
    SELECT authority.operation_id,
           authority.request_fingerprint,
           authority.authority_epoch,
           authority.fence
      INTO canonical_authority_operation_id,
           canonical_authority_request_fingerprint,
           canonical_authority_epoch,
           canonical_authority_fence
        FROM authorization_operation_receipts receipt
        JOIN authorization_admission_results admission
          ON admission.community_id = receipt.community_id
         AND admission.operation_id = receipt.operation_id
         AND admission.request_fingerprint = receipt.request_fingerprint
        JOIN protected_object_authority authority
          ON authority.community_id = admission.community_id
         AND authority.object_kind = admission.object_kind
         AND authority.object_key = admission.object_key
        JOIN authorization_authority_epochs epoch
          ON epoch.community_id = authority.community_id
         AND epoch.object_kind = authority.object_kind
         AND epoch.object_key = authority.object_key
         AND epoch.authority_epoch = authority.authority_epoch
         AND epoch.fence = authority.fence
         AND epoch.operation_id = authority.operation_id
         AND epoch.request_fingerprint = authority.request_fingerprint
        JOIN authorization_operation_receipts authority_receipt
          ON authority_receipt.community_id = authority.community_id
         AND authority_receipt.operation_id = authority.operation_id
         AND authority_receipt.request_fingerprint = authority.request_fingerprint
        WHERE receipt.community_id = NEW.community_id
          AND receipt.operation_id = NEW.operation_id
          AND receipt.request_fingerprint = NEW.request_fingerprint
          -- First-use enrollment may atomically carry this protected effect;
          -- every later established-binding mutation uses kind 11.
          AND receipt.operation_kind IN (1, 11)
          AND receipt.outcome_code = 1
          AND admission.object_kind = NEW.object_kind
          AND admission.object_key = NEW.object_key
          AND admission.application_type = NEW.application_type
          AND admission.application_version = NEW.application_version
          AND admission.application_effect_digest = NEW.application_effect_digest
          AND admission.application_result_digest = NEW.publication_result_digest
          AND authority_receipt.operation_kind IN (1, 11)
          AND authority_receipt.outcome_code = 1
        FOR SHARE OF authority, epoch, authority_receipt;

    IF canonical_authority_operation_id IS NULL THEN
        RAISE EXCEPTION 'protected publication projection requires exact applied receipt'
            USING ERRCODE = 'foreign_key_violation',
                  CONSTRAINT = 'protected_publication_projection_receipt';
    END IF;

    UPDATE protected_publication_projection_outbox projection
       SET authority_operation_id = canonical_authority_operation_id,
           authority_request_fingerprint = canonical_authority_request_fingerprint,
           authority_epoch = canonical_authority_epoch,
           authority_fence = canonical_authority_fence
     WHERE projection.community_id = NEW.community_id
       AND projection.operation_id = NEW.operation_id
       AND projection.projection_kind = NEW.projection_kind
       AND projection.authority_operation_id IS NULL
       AND projection.authority_request_fingerprint IS NULL
       AND projection.authority_epoch IS NULL
       AND projection.authority_fence IS NULL;

    IF NOT EXISTS (
        SELECT 1
          FROM protected_publication_projection_outbox projection
         WHERE projection.community_id = NEW.community_id
           AND projection.operation_id = NEW.operation_id
           AND projection.projection_kind = NEW.projection_kind
           AND projection.authority_operation_id = canonical_authority_operation_id
           AND projection.authority_request_fingerprint =
               canonical_authority_request_fingerprint
           AND projection.authority_epoch = canonical_authority_epoch
           AND projection.authority_fence = canonical_authority_fence
    ) THEN
        RAISE EXCEPTION 'protected publication projection authority binding is invalid'
            USING ERRCODE = 'foreign_key_violation',
                  CONSTRAINT = 'protected_publication_projection_receipt';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER protected_publication_projection_receipt
    AFTER INSERT ON protected_publication_projection_outbox
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION protected_publication_projection_receipt_guard_v1();

CREATE FUNCTION protected_publication_projection_state_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    -- The deferred receipt guard owns the sole transition from an unbound
    -- in-transaction row to its immutable canonical authority snapshot.
    IF pg_trigger_depth() = 2
        AND OLD.authority_operation_id IS NULL
        AND OLD.authority_request_fingerprint IS NULL
        AND OLD.authority_epoch IS NULL
        AND OLD.authority_fence IS NULL
        AND NEW.authority_operation_id IS NOT NULL
        AND NEW.authority_request_fingerprint IS NOT NULL
        AND NEW.authority_epoch IS NOT NULL
        AND NEW.authority_fence IS NOT NULL
        AND NEW.community_id IS NOT DISTINCT FROM OLD.community_id
        AND NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        AND NEW.request_fingerprint IS NOT DISTINCT FROM OLD.request_fingerprint
        AND NEW.publication_result_digest IS NOT DISTINCT FROM OLD.publication_result_digest
        AND NEW.object_kind IS NOT DISTINCT FROM OLD.object_kind
        AND NEW.object_key IS NOT DISTINCT FROM OLD.object_key
        AND NEW.application_type IS NOT DISTINCT FROM OLD.application_type
        AND NEW.application_version IS NOT DISTINCT FROM OLD.application_version
        AND NEW.application_effect_digest IS NOT DISTINCT FROM OLD.application_effect_digest
        AND NEW.projection_kind IS NOT DISTINCT FROM OLD.projection_kind
        AND NEW.projection_key IS NOT DISTINCT FROM OLD.projection_key
        AND NEW.staged_object_key IS NOT DISTINCT FROM OLD.staged_object_key
        AND NEW.payload_digest IS NOT DISTINCT FROM OLD.payload_digest
        AND NEW.delivery_state IS NOT DISTINCT FROM OLD.delivery_state
        AND NEW.attempt_count IS NOT DISTINCT FROM OLD.attempt_count
        AND NEW.next_attempt_at IS NOT DISTINCT FROM OLD.next_attempt_at
        AND NEW.last_attempt_at IS NOT DISTINCT FROM OLD.last_attempt_at
        AND NEW.delivered_at IS NOT DISTINCT FROM OLD.delivered_at
        AND NEW.failure_code IS NOT DISTINCT FROM OLD.failure_code
        AND NEW.created_at IS NOT DISTINCT FROM OLD.created_at
        AND NEW.publication_sequence IS NOT DISTINCT FROM OLD.publication_sequence
    THEN
        RETURN NEW;
    END IF;

    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
        OR NEW.request_fingerprint IS DISTINCT FROM OLD.request_fingerprint
        OR NEW.publication_result_digest IS DISTINCT FROM OLD.publication_result_digest
        OR NEW.object_kind IS DISTINCT FROM OLD.object_kind
        OR NEW.object_key IS DISTINCT FROM OLD.object_key
        OR NEW.application_type IS DISTINCT FROM OLD.application_type
        OR NEW.application_version IS DISTINCT FROM OLD.application_version
        OR NEW.application_effect_digest IS DISTINCT FROM OLD.application_effect_digest
        OR NEW.authority_operation_id IS DISTINCT FROM OLD.authority_operation_id
        OR NEW.authority_request_fingerprint IS DISTINCT FROM OLD.authority_request_fingerprint
        OR NEW.authority_epoch IS DISTINCT FROM OLD.authority_epoch
        OR NEW.authority_fence IS DISTINCT FROM OLD.authority_fence
        OR NEW.projection_kind IS DISTINCT FROM OLD.projection_kind
        OR NEW.projection_key IS DISTINCT FROM OLD.projection_key
        OR NEW.staged_object_key IS DISTINCT FROM OLD.staged_object_key
        OR NEW.payload_digest IS DISTINCT FROM OLD.payload_digest
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
        OR NEW.publication_sequence IS DISTINCT FROM OLD.publication_sequence
        OR OLD.delivery_state <> 1
        OR NEW.delivery_state NOT IN (1, 2, 3)
        OR NEW.attempt_count <> OLD.attempt_count + 1
        OR NEW.next_attempt_at < OLD.next_attempt_at
        OR (NEW.delivery_state = 1 AND NEW.failure_code NOT IN (1, 2))
        OR (
            NEW.delivery_state = 2
            AND EXISTS (
                SELECT 1
                FROM protected_publication_projection_outbox earlier
                WHERE earlier.community_id = OLD.community_id
                  AND earlier.object_kind = OLD.object_kind
                  AND earlier.object_key = OLD.object_key
                  AND earlier.projection_kind = OLD.projection_kind
                  AND earlier.projection_key = OLD.projection_key
                  AND earlier.publication_sequence < OLD.publication_sequence
                  AND earlier.delivery_state = 1
            )
        )
    THEN
        RAISE EXCEPTION 'protected publication projection transition is invalid'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'protected_publication_projection_transition';
    END IF;
    NEW.last_attempt_at := transaction_timestamp();
    IF NEW.delivery_state = 2 THEN
        NEW.delivered_at := transaction_timestamp();
        NEW.failure_code := 0;
    ELSE
        NEW.delivered_at := NULL;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER protected_publication_projection_state
    BEFORE UPDATE ON protected_publication_projection_outbox
    FOR EACH ROW EXECUTE FUNCTION protected_publication_projection_state_guard_v1();
CREATE TRIGGER protected_publication_projection_no_delete
    BEFORE DELETE ON protected_publication_projection_outbox
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER protected_publication_projection_no_truncate
    BEFORE TRUNCATE ON protected_publication_projection_outbox
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();
