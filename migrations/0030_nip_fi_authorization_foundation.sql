-- Provider-free NIP-FI authorization, audit, fencing, and restore foundation.
--
-- There is no provider registry/SPI/profile/evidence table, durable lease or
-- audio admission ledger, 30382 projection, delivery queue, exporter claim,
-- acknowledgement, retry scheduler, or online retention/compaction workflow.

-- Durable one-way activation marker and current domain invalidation generation.
CREATE TABLE authorization_invalidation_domains (
    community_id UUID NOT NULL PRIMARY KEY REFERENCES communities(id),
    current_generation BIGINT NOT NULL CHECK (current_generation >= 0),
    activated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp()
);

-- Closed selectors: 1 principal, 2 Nostr key, 3 binding, 4 session, 5 domain,
-- 6 configuration revision, 7 delegated relationship. Kind 7 is the
-- authoritative relationship identity/revision coordinate; it contains no
-- provider vocabulary.
CREATE TABLE authorization_invalidation_floors (
    community_id UUID NOT NULL REFERENCES communities(id),
    selector_kind SMALLINT NOT NULL CHECK (selector_kind IN (1, 2, 3, 4, 5, 6, 7)),
    selector_fingerprint BYTEA NOT NULL CHECK (octet_length(selector_fingerprint) = 32),
    floor_generation BIGINT NOT NULL CHECK (floor_generation > 0),
    binding_version_floor BIGINT CHECK (binding_version_floor IS NULL OR binding_version_floor > 0),
    relationship_revision_floor BIGINT CHECK (
        relationship_revision_floor IS NULL OR relationship_revision_floor > 0
    ),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, selector_kind, selector_fingerprint),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (
        (selector_kind = 3
            AND binding_version_floor IS NOT NULL
            AND relationship_revision_floor IS NULL)
        OR (selector_kind = 7
            AND binding_version_floor IS NULL
            AND relationship_revision_floor IS NOT NULL)
        OR (selector_kind NOT IN (3, 7)
            AND binding_version_floor IS NULL
            AND relationship_revision_floor IS NULL)
    )
);

-- Protected-object kinds: 1 domain, 2 channel, 3 repository, 4 media,
-- 5 moderation target, 6 audio session. Kind 7 is retired: current binding
-- status is connection-local evidence and never a durable protected object.
CREATE TABLE authorization_authority_epochs (
    community_id UUID NOT NULL REFERENCES communities(id),
    object_kind SMALLINT NOT NULL CHECK (object_kind IN (1, 2, 3, 4, 5, 6)),
    object_key BYTEA NOT NULL CHECK (octet_length(object_key) = 32),
    authority_epoch BIGINT NOT NULL CHECK (authority_epoch > 0),
    fence BYTEA NOT NULL CHECK (
        octet_length(fence) = 32 AND fence <> decode(repeat('00', 32), 'hex')
    ),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, object_kind, object_key),
    UNIQUE (
        community_id,
        object_kind,
        object_key,
        authority_epoch,
        fence,
        operation_id,
        request_fingerprint
    ),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED
);

-- Direct-final current authority for a protected object. The authorization
-- lease itself is sealed in memory and dies on restart; this durable row is the
-- exact source re-fenced immediately before a protected mutation or emission.
CREATE TABLE protected_object_authority (
    community_id UUID NOT NULL REFERENCES communities(id),
    object_kind SMALLINT NOT NULL CHECK (object_kind IN (1, 2, 3, 4, 5, 6)),
    object_key BYTEA NOT NULL CHECK (octet_length(object_key) = 32),
    capability SMALLINT NOT NULL CHECK (
        capability IN (
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
            15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
            28, 29
        )
    ),
    actor_pubkey BYTEA NOT NULL CHECK (octet_length(actor_pubkey) = 32),
    owner_pubkey BYTEA CHECK (owner_pubkey IS NULL OR octet_length(owner_pubkey) = 32),
    binding_id UUID NOT NULL,
    binding_version BIGINT NOT NULL CHECK (binding_version > 0),
    delegated_relationship_id UUID,
    delegated_relationship_revision BIGINT CHECK (
        delegated_relationship_revision IS NULL OR delegated_relationship_revision > 0
    ),
    delegation_conditions_fingerprint BYTEA CHECK (
        delegation_conditions_fingerprint IS NULL
        OR octet_length(delegation_conditions_fingerprint) = 32
    ),
    policy_revision BIGINT NOT NULL CHECK (policy_revision > 0),
    invalidation_generation BIGINT NOT NULL CHECK (invalidation_generation >= 0),
    authority_epoch BIGINT NOT NULL CHECK (authority_epoch > 0),
    fence BYTEA NOT NULL CHECK (
        octet_length(fence) = 32 AND fence <> decode(repeat('00', 32), 'hex')
    ),
    issued_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    PRIMARY KEY (community_id, object_kind, object_key),
    CONSTRAINT protected_object_authority_delegated_relationship_non_nil CHECK (
        delegated_relationship_id IS NULL
        OR delegated_relationship_id <> '00000000-0000-0000-0000-000000000000'::uuid
    ),
    FOREIGN KEY (community_id, binding_id, binding_version)
        REFERENCES identity_bindings (community_id, binding_id, binding_version)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (
        community_id,
        object_kind,
        object_key,
        authority_epoch,
        fence,
        operation_id,
        request_fingerprint
    ) REFERENCES authorization_authority_epochs (
        community_id,
        object_kind,
        object_key,
        authority_epoch,
        fence,
        operation_id,
        request_fingerprint
    ) DEFERRABLE INITIALLY DEFERRED,
    CHECK (issued_at < expires_at),
    CHECK (
        (owner_pubkey IS NULL
            AND delegated_relationship_id IS NULL
            AND delegated_relationship_revision IS NULL
            AND delegation_conditions_fingerprint IS NULL)
        OR (owner_pubkey IS NOT NULL
            AND delegated_relationship_id IS NOT NULL
            AND delegated_relationship_revision IS NOT NULL
            AND delegation_conditions_fingerprint IS NOT NULL)
    )
);

-- Explicit immutable-capacity policy required by Enforce mode. Hard ceilings
-- match buzz-auth; installation limits must be sized explicitly below them.
-- V1 has no online pruning/export/reset workflow.
CREATE TABLE authorization_event_capacity (
    community_id UUID NOT NULL PRIMARY KEY REFERENCES communities(id),
    max_events_per_domain BIGINT NOT NULL CONSTRAINT authorization_event_capacity_max_events CHECK (
        max_events_per_domain BETWEEN 1 AND 10000
    ),
    max_bytes_per_domain BIGINT NOT NULL CONSTRAINT authorization_event_capacity_max_bytes CHECK (
        max_bytes_per_domain BETWEEN 1 AND 16777216
    ),
    max_envelope_bytes INTEGER NOT NULL CONSTRAINT authorization_event_capacity_max_envelope CHECK (
        max_envelope_bytes BETWEEN 1 AND 16384
    ),
    retained_event_count BIGINT NOT NULL DEFAULT 0 CHECK (retained_event_count >= 0),
    retained_envelope_bytes BIGINT NOT NULL DEFAULT 0 CHECK (retained_envelope_bytes >= 0),
    -- 1 healthy, 2 audit unavailable/exhausted. Recovery/reset is not a V1
    -- online workflow; enabled runtime latches failure when insertion aborts.
    health_state SMALLINT NOT NULL DEFAULT 1 CHECK (health_state IN (1, 2)),
    failure_code SMALLINT CHECK (failure_code IS NULL OR failure_code IN (1, 2, 3)),
    failure_observed_at TIMESTAMPTZ,
    configured_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    CHECK (max_envelope_bytes <= max_bytes_per_domain),
    CHECK (retained_event_count <= max_events_per_domain),
    CHECK (retained_envelope_bytes <= max_bytes_per_domain),
    CHECK (
        (health_state = 1 AND failure_code IS NULL AND failure_observed_at IS NULL)
        OR (health_state = 2 AND failure_code IS NOT NULL AND failure_observed_at IS NOT NULL)
    )
);

-- Durable versioned pseudonymous authorization envelope. event_kind:
-- 1 enrolled, 2 revoked, 3 rotated, 4 recovered, 5 principal enabled,
-- 6 retired, 7 principal disabled, 8 admission lost, 9 operator denied,
-- 10 protected allowed, 11 protected denied, 14 invalidation advanced.
-- Kinds 12 and 13 are retired: kind 24244 publication/withdrawal is ephemeral
-- connection state and never a durable authorization event.
CREATE TABLE authorization_events (
    community_id UUID NOT NULL REFERENCES communities(id),
    event_id UUID NOT NULL,
    schema_version SMALLINT NOT NULL DEFAULT 1 CHECK (schema_version = 1),
    event_kind SMALLINT NOT NULL CHECK (
        event_kind IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14)
    ),
    outcome_code SMALLINT NOT NULL CHECK (outcome_code IN (1, 2, 3, 4, 5)),
    reason_code SMALLINT NOT NULL CHECK (
        reason_code IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)
    ),
    actor_kind SMALLINT NOT NULL CHECK (actor_kind IN (1, 2, 3, 4)),
    actor_fingerprint BYTEA CHECK (
        actor_fingerprint IS NULL OR octet_length(actor_fingerprint) = 32
    ),
    subject_fingerprint BYTEA CHECK (
        subject_fingerprint IS NULL OR octet_length(subject_fingerprint) = 32
    ),
    -- Always retains attempted operation identity. Only unresolved pre-auth
    -- event kind 9 omits the canonical receipt fingerprint; authenticated
    -- OperatorDenied events remain linked to their exact canonical receipt.
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA CHECK (
        request_fingerprint IS NULL OR octet_length(request_fingerprint) = 32
    ),
    correlation_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    canonical_envelope BYTEA NOT NULL CONSTRAINT authorization_events_envelope_size CHECK (
        octet_length(canonical_envelope) BETWEEN 1 AND 16384
    ),
    envelope_digest BYTEA NOT NULL CHECK (octet_length(envelope_digest) = 32),
    PRIMARY KEY (community_id, event_id),
    UNIQUE (community_id, event_id, operation_id),
    UNIQUE (community_id, event_id, event_kind, operation_id),
    UNIQUE (community_id, operation_id, event_kind, attempt_id),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (event_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (correlation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (attempt_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (
        (actor_kind = 4 AND event_kind = 9 AND request_fingerprint IS NULL)
        OR (actor_kind IN (1, 2, 3) AND request_fingerprint IS NOT NULL)
    ),
    CHECK (
        (actor_kind = 4 AND actor_fingerprint IS NULL AND subject_fingerprint IS NULL)
        OR (actor_kind IN (1, 2, 3) AND actor_fingerprint IS NOT NULL)
    )
);

-- Credential-free pre-authentication denial attempts. The five-column key is
-- exact replay identity; no row or FK occupies canonical operation/result,
-- effect, authority, approval, or consumption state.
CREATE TABLE authorization_authentication_denial_attempts (
    community_id UUID NOT NULL REFERENCES communities(id),
    operation_id UUID NOT NULL,
    correlation_id UUID NOT NULL,
    semantic_fingerprint BYTEA NOT NULL CHECK (octet_length(semantic_fingerprint) = 32),
    denial_reason SMALLINT NOT NULL CHECK (denial_reason IN (1, 2, 3)),
    expected_revision BIGINT NOT NULL CHECK (expected_revision > 0),
    action SMALLINT NOT NULL CHECK (action IN (1, 2, 3, 4, 5, 6, 7, 8)),
    reason_code SMALLINT NOT NULL CHECK (
        reason_code IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)
    ),
    audit_event_id UUID NOT NULL,
    audit_event_kind SMALLINT NOT NULL DEFAULT 9 CHECK (audit_event_kind = 9),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (
        community_id,
        operation_id,
        correlation_id,
        semantic_fingerprint,
        denial_reason
    ),
    UNIQUE (community_id, audit_event_id),
    FOREIGN KEY (community_id, audit_event_id, audit_event_kind, operation_id)
        REFERENCES authorization_events (community_id, event_id, event_kind, operation_id)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (correlation_id <> '00000000-0000-0000-0000-000000000000'::uuid)
);

-- Exact per-operation authority-version attribution for restore. Empty
-- manifests are valid; every stored component must advance strictly.
CREATE TABLE authorization_operation_version_delta_manifests (
    community_id UUID NOT NULL REFERENCES communities(id),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    component_count INTEGER NOT NULL CHECK (component_count BETWEEN 0 AND 1024),
    before_digest BYTEA NOT NULL CHECK (octet_length(before_digest) = 32),
    after_digest BYTEA NOT NULL CHECK (octet_length(after_digest) = 32),
    manifest_digest BYTEA NOT NULL CHECK (octet_length(manifest_digest) = 32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, operation_id),
    UNIQUE (community_id, operation_id, request_fingerprint),
    FOREIGN KEY (community_id, operation_id, request_fingerprint)
        REFERENCES authorization_operation_receipts
            (community_id, operation_id, request_fingerprint)
        DEFERRABLE INITIALLY DEFERRED
);

-- component_kind: 1 binding version, 2 policy revision,
-- 3 invalidation generation, 4 authority epoch, 6 delegated-relationship
-- revision, 7 lifecycle-selector generation. Kind 5 is retired with durable
-- client-status revisions; retained kinds keep their original identities.
CREATE TABLE authorization_operation_version_deltas (
    community_id UUID NOT NULL REFERENCES communities(id),
    operation_id UUID NOT NULL,
    component_kind SMALLINT NOT NULL CHECK (component_kind IN (1, 2, 3, 4, 6, 7)),
    component_key BYTEA NOT NULL CHECK (octet_length(component_key) = 32),
    before_version BIGINT NOT NULL CHECK (before_version >= 0),
    after_version BIGINT NOT NULL,
    component_digest BYTEA NOT NULL CHECK (octet_length(component_digest) = 32),
    PRIMARY KEY (community_id, operation_id, component_kind, component_key),
    FOREIGN KEY (community_id, operation_id)
        REFERENCES authorization_operation_version_delta_manifests
            (community_id, operation_id),
    CHECK (after_version > before_version)
);

CREATE FUNCTION authorization_event_capacity_before_insert_v1() RETURNS TRIGGER AS $$
DECLARE
    policy authorization_event_capacity%ROWTYPE;
    envelope_bytes BIGINT;
BEGIN
    SELECT * INTO policy
    FROM authorization_event_capacity
    WHERE community_id = NEW.community_id
    FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'authorization event capacity policy missing'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_policy_required';
    END IF;
    IF policy.health_state <> 1 THEN
        RAISE EXCEPTION 'authorization audit is unavailable'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_health';
    END IF;

    envelope_bytes := octet_length(NEW.canonical_envelope);
    IF envelope_bytes > policy.max_envelope_bytes
        OR policy.retained_event_count + 1 > policy.max_events_per_domain
        OR policy.retained_envelope_bytes + envelope_bytes > policy.max_bytes_per_domain
    THEN
        -- The INSERT and protected mutation abort together. The runtime maps
        -- this stable constraint to typed CapacityExhausted and latches audit
        -- health outside the rolled-back transaction.
        RAISE EXCEPTION 'authorization event capacity exhausted'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_exhausted';
    END IF;

    UPDATE authorization_event_capacity
    SET retained_event_count = retained_event_count + 1,
        retained_envelope_bytes = retained_envelope_bytes + envelope_bytes,
        updated_at = transaction_timestamp()
    WHERE community_id = NEW.community_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_events_capacity
    BEFORE INSERT ON authorization_events
    FOR EACH ROW EXECUTE FUNCTION authorization_event_capacity_before_insert_v1();

CREATE FUNCTION authorization_invalidation_domain_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.activated_at IS DISTINCT FROM OLD.activated_at
        OR NEW.current_generation <= OLD.current_generation
        OR NEW.updated_at <= OLD.updated_at
    THEN
        RAISE EXCEPTION 'authorization invalidation activation/generation cannot move backward'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_invalidation_domains_monotonic
    BEFORE UPDATE ON authorization_invalidation_domains
    FOR EACH ROW EXECUTE FUNCTION authorization_invalidation_domain_guard_v1();
CREATE TRIGGER authorization_invalidation_domains_no_delete
    BEFORE DELETE ON authorization_invalidation_domains
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_invalidation_domains_no_truncate
    BEFORE TRUNCATE ON authorization_invalidation_domains
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE FUNCTION authorization_invalidation_floor_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.selector_kind IS DISTINCT FROM OLD.selector_kind
        OR NEW.selector_fingerprint IS DISTINCT FROM OLD.selector_fingerprint
        OR NEW.floor_generation < OLD.floor_generation
        OR COALESCE(NEW.binding_version_floor, 0) < COALESCE(OLD.binding_version_floor, 0)
        OR COALESCE(NEW.relationship_revision_floor, 0)
            < COALESCE(OLD.relationship_revision_floor, 0)
        OR (
            NEW.floor_generation = OLD.floor_generation
            AND COALESCE(NEW.binding_version_floor, 0)
                = COALESCE(OLD.binding_version_floor, 0)
            AND COALESCE(NEW.relationship_revision_floor, 0)
                = COALESCE(OLD.relationship_revision_floor, 0)
        )
        OR NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        OR NEW.updated_at <= OLD.updated_at
    THEN
        RAISE EXCEPTION 'authorization invalidation floor cannot move backward'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_invalidation_floors_monotonic
    BEFORE UPDATE ON authorization_invalidation_floors
    FOR EACH ROW EXECUTE FUNCTION authorization_invalidation_floor_guard_v1();
CREATE TRIGGER authorization_invalidation_floors_no_delete
    BEFORE DELETE ON authorization_invalidation_floors
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_invalidation_floors_no_truncate
    BEFORE TRUNCATE ON authorization_invalidation_floors
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE FUNCTION authorization_authority_epoch_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.object_kind IS DISTINCT FROM OLD.object_kind
        OR NEW.object_key IS DISTINCT FROM OLD.object_key
        OR NEW.authority_epoch <= OLD.authority_epoch
        OR NEW.fence IS NOT DISTINCT FROM OLD.fence
        OR NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        OR NEW.updated_at <= OLD.updated_at
    THEN
        RAISE EXCEPTION 'authorization authority epoch cannot move backward'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_authority_epochs_monotonic
    BEFORE UPDATE ON authorization_authority_epochs
    FOR EACH ROW EXECUTE FUNCTION authorization_authority_epoch_guard_v1();
CREATE TRIGGER authorization_authority_epochs_no_delete
    BEFORE DELETE ON authorization_authority_epochs
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_authority_epochs_no_truncate
    BEFORE TRUNCATE ON authorization_authority_epochs
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE FUNCTION authorization_event_capacity_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.max_events_per_domain IS DISTINCT FROM OLD.max_events_per_domain
        OR NEW.max_bytes_per_domain IS DISTINCT FROM OLD.max_bytes_per_domain
        OR NEW.max_envelope_bytes IS DISTINCT FROM OLD.max_envelope_bytes
        OR NEW.configured_at IS DISTINCT FROM OLD.configured_at
        OR NEW.retained_event_count < OLD.retained_event_count
        OR NEW.retained_envelope_bytes < OLD.retained_envelope_bytes
        OR NEW.updated_at < OLD.updated_at
        OR (OLD.health_state = 2 AND (
            NEW.health_state <> 2
            OR NEW.failure_code IS DISTINCT FROM OLD.failure_code
            OR NEW.failure_observed_at IS DISTINCT FROM OLD.failure_observed_at
        ))
        OR (OLD.health_state = 1 AND NEW.health_state = 1 AND (
            NEW.failure_code IS NOT NULL OR NEW.failure_observed_at IS NOT NULL
        ))
    THEN
        RAISE EXCEPTION 'authorization event capacity cannot be reset online'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION protected_object_authority_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.object_kind IS DISTINCT FROM OLD.object_kind
        OR NEW.object_key IS DISTINCT FROM OLD.object_key
        OR NEW.authority_epoch <= OLD.authority_epoch
        OR NEW.fence IS NOT DISTINCT FROM OLD.fence
        OR NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        OR NEW.issued_at <= OLD.issued_at
    THEN
        RAISE EXCEPTION 'protected authority replacement requires a new operation and epoch'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_event_capacity_monotonic
    BEFORE UPDATE ON authorization_event_capacity
    FOR EACH ROW EXECUTE FUNCTION authorization_event_capacity_guard_v1();
CREATE TRIGGER authorization_event_capacity_no_delete
    BEFORE DELETE ON authorization_event_capacity
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_event_capacity_no_truncate
    BEFORE TRUNCATE ON authorization_event_capacity
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER authorization_events_immutable
    BEFORE UPDATE OR DELETE ON authorization_events
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_events_no_truncate
    BEFORE TRUNCATE ON authorization_events
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER authorization_authentication_denial_attempts_immutable
    BEFORE UPDATE OR DELETE ON authorization_authentication_denial_attempts
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_authentication_denial_attempts_no_truncate
    BEFORE TRUNCATE ON authorization_authentication_denial_attempts
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE FUNCTION authorization_operation_version_delta_cardinality_guard_v1()
RETURNS TRIGGER AS $$
DECLARE
    manifest authorization_operation_version_delta_manifests%ROWTYPE;
    actual_component_count BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'authorization_operation_version_delta_manifests' THEN
        manifest := NEW;
    ELSE
        SELECT * INTO STRICT manifest
        FROM authorization_operation_version_delta_manifests
        WHERE community_id = NEW.community_id
          AND operation_id = NEW.operation_id
        FOR NO KEY UPDATE;
    END IF;

    SELECT count(*) INTO actual_component_count
    FROM authorization_operation_version_deltas
    WHERE community_id = manifest.community_id
      AND operation_id = manifest.operation_id;

    IF actual_component_count <> manifest.component_count THEN
        RAISE EXCEPTION 'operation version manifest declares % components, found %',
            manifest.component_count, actual_component_count
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_operation_version_delta_cardinality';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER authorization_operation_version_delta_manifest_cardinality
    AFTER INSERT ON authorization_operation_version_delta_manifests
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operation_version_delta_cardinality_guard_v1();
CREATE CONSTRAINT TRIGGER authorization_operation_version_delta_component_cardinality
    AFTER INSERT ON authorization_operation_version_deltas
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operation_version_delta_cardinality_guard_v1();

CREATE TRIGGER authorization_operation_version_delta_manifests_immutable
    BEFORE UPDATE OR DELETE ON authorization_operation_version_delta_manifests
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_operation_version_delta_manifests_no_truncate
    BEFORE TRUNCATE ON authorization_operation_version_delta_manifests
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER authorization_operation_version_deltas_immutable
    BEFORE UPDATE OR DELETE ON authorization_operation_version_deltas
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_operation_version_deltas_no_truncate
    BEFORE TRUNCATE ON authorization_operation_version_deltas
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER protected_object_authority_no_delete
    BEFORE DELETE ON protected_object_authority
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER protected_object_authority_no_truncate
    BEFORE TRUNCATE ON protected_object_authority
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();
CREATE TRIGGER protected_object_authority_strict_replacement
    BEFORE UPDATE ON protected_object_authority
    FOR EACH ROW EXECUTE FUNCTION protected_object_authority_guard_v1();
