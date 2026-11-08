-- Provider-free NIP-FI core identity and base-lifecycle foundation.
--
-- This is direct-final fresh-schema DDL. It intentionally does not replay a
-- historical uid/backfill/ALTER sequence.
--
-- Scope is NIP-FI *core* only. Base lifecycle is exactly retire, revoke, and
-- rotate (NIP-FI.md "Base lifecycle"). The extended NIP-FI-LIFECYCLE surface
-- (disabled identities, pending-replacement lineage, and their provision,
-- disable, recover, enable, and admission-loss transitions) is deferred to a
-- later migration owned by the FI-LIFECYCLE PR, per NIP-FI-MODEL.md: "NIP-FI-
-- LIFECYCLE adds disabled identities and pending replacement lineage." So the
-- closed vocabularies below are the core subset:
--   transition/operation kinds: 1 enroll, 3 retire, 5 revoke, 6 rotate;
--   lifecycle selector kinds:   1 retired pair (P), 3 revoked key (Y).
-- A later migration widens these vocabularies additively; nothing here presumes
-- a single global issuer — identity is issuer-qualified (iss, sub).

-- The sole idempotency/result root shared by identity base lifecycle,
-- protected operations, and invalidation. Pre-authentication denials never
-- write this table. ExactReplay and IntentConflict are read-time observations,
-- not persisted outcomes.
CREATE TABLE authorization_operation_receipts (
    community_id UUID NOT NULL REFERENCES communities(id),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    -- Core operation kinds: 1 enroll, 3 retire, 5 revoke, 6 rotate,
    -- 11 protected mutation, 12 invalidation. Extended lifecycle kinds
    -- (2 provision, 4 disable, 7 recover, 8 enable, 9 admission loss) and
    -- 10 operator are introduced by their owning later migrations.
    operation_kind SMALLINT NOT NULL CHECK (
        operation_kind IN (1, 3, 5, 6, 11, 12)
    ),
    actor_fingerprint BYTEA NOT NULL CHECK (octet_length(actor_fingerprint) = 32),
    -- 1 applied, 2 denied, 3 no-op.
    outcome_code SMALLINT NOT NULL CHECK (outcome_code IN (1, 2, 3)),
    result_digest BYTEA NOT NULL CHECK (octet_length(result_digest) = 32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, operation_id),
    UNIQUE (community_id, operation_id, request_fingerprint),
    UNIQUE (
        community_id,
        operation_id,
        request_fingerprint,
        operation_kind,
        outcome_code
    ),
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid)
);

-- Immutable monotonic local policy revisions. Enrollment modes are the closed
-- provider-free V1 set: 1 attested-key, 2 provisioned, 3 risk-labelled TOFU.
CREATE TABLE identity_enrollment_policies (
    community_id UUID NOT NULL REFERENCES communities(id),
    policy_revision BIGINT NOT NULL CHECK (policy_revision > 0),
    enrollment_mode SMALLINT NOT NULL CHECK (enrollment_mode IN (1, 2, 3)),
    policy_digest BYTEA NOT NULL CHECK (octet_length(policy_digest) = 32),
    effective_at TIMESTAMPTZ NOT NULL,
    -- Optional local binding-policy expiry. Federated token `exp` MUST NOT be
    -- copied here: token lifetime bounds an authorization lease, not this
    -- durable binding generation.
    expires_at TIMESTAMPTZ,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, policy_revision),
    CHECK (expires_at IS NULL OR effective_at < expires_at)
);

-- One row is one immutable binding generation. binding_version is allocated
-- from one non-cycling PostgreSQL identity sequence and is never changed or
-- reused. Explicit lifecycle may only retire the generation; X/Y denial
-- semantics live in immutable selector facts below, not alternate row states.
CREATE TABLE identity_bindings (
    community_id UUID NOT NULL REFERENCES communities(id),
    binding_id UUID NOT NULL,
    binding_version BIGINT GENERATED ALWAYS AS IDENTITY (
        START WITH 1 INCREMENT BY 1 NO MINVALUE NO MAXVALUE CACHE 1 NO CYCLE
    ),
    issuer TEXT COLLATE "C" NOT NULL CHECK (octet_length(issuer) BETWEEN 1 AND 2048),
    subject TEXT COLLATE "C" NOT NULL CHECK (octet_length(subject) BETWEEN 1 AND 2048),
    principal_fingerprint BYTEA NOT NULL CHECK (octet_length(principal_fingerprint) = 32),
    event_author_pubkey BYTEA NOT NULL CHECK (octet_length(event_author_pubkey) = 32),
    -- 1 active, 2 retired.
    binding_state SMALLINT NOT NULL CHECK (binding_state IN (1, 2)),
    lifecycle_revision BIGINT NOT NULL CHECK (lifecycle_revision IN (1, 2)),
    -- 1 attested-key, 2 provisioned, 3 risk-labelled TOFU.
    binding_provenance SMALLINT NOT NULL CHECK (binding_provenance IN (1, 2, 3)),
    policy_revision BIGINT NOT NULL CHECK (policy_revision > 0),
    -- Canonical evidence for the selected provenance. This is an assertion
    -- digest for attested/TOFU admission and a provisioning receipt digest for
    -- separately provisioned admission; it never stores credential bytes.
    enrollment_evidence_digest BYTEA NOT NULL CHECK (
        octet_length(enrollment_evidence_digest) = 32
    ),
    expires_at TIMESTAMPTZ,
    birth_history_id UUID NOT NULL,
    creation_operation_id UUID NOT NULL,
    creation_request_fingerprint BYTEA NOT NULL CHECK (
        octet_length(creation_request_fingerprint) = 32
    ),
    retirement_history_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, binding_id),
    UNIQUE (community_id, binding_version),
    UNIQUE (community_id, binding_id, binding_version),
    FOREIGN KEY (community_id, policy_revision)
        REFERENCES identity_enrollment_policies
            (community_id, policy_revision),
    CHECK (binding_version > 0),
    CHECK (binding_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (birth_history_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (creation_operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (expires_at IS NULL OR created_at < expires_at),
    CHECK (
        (binding_state = 1 AND lifecycle_revision = 1 AND retirement_history_id IS NULL)
        OR (binding_state = 2 AND lifecycle_revision = 2 AND retirement_history_id IS NOT NULL)
    )
);

-- State 1 is Active. Expiry is evaluated with authoritative PostgreSQL time
-- at read/finalization and is exclusive; it cannot appear in an index predicate.
CREATE UNIQUE INDEX identity_bindings_active_principal
    ON identity_bindings (community_id, issuer, subject)
    WHERE binding_state = 1;
CREATE INDEX identity_bindings_principal_fingerprint_lookup
    ON identity_bindings (community_id, principal_fingerprint)
    WHERE binding_state = 1;
CREATE UNIQUE INDEX identity_bindings_active_event_author
    ON identity_bindings (community_id, event_author_pubkey)
    WHERE binding_state = 1;
CREATE INDEX identity_bindings_current_lookup
    ON identity_bindings (community_id, event_author_pubkey, binding_state, expires_at);

-- The one canonical immutable lifecycle transition row for a successful or
-- no-op lifecycle operation. A transition can name an old generation, a new
-- successor generation, both (Rotate), or neither (a semantic no-op). It is not
-- a second result/effect engine: the shared receipt remains the sole persisted
-- operation outcome. Core transition kinds only: 1 enroll, 3 retire, 5 revoke,
-- 6 rotate.
CREATE TABLE identity_lifecycle_history (
    community_id UUID NOT NULL REFERENCES communities(id),
    history_id UUID NOT NULL,
    transition_kind SMALLINT NOT NULL CHECK (
        transition_kind IN (1, 3, 5, 6)
    ),
    -- Matches the shared receipt: 1 applied, 3 no-op.
    outcome_code SMALLINT NOT NULL CHECK (outcome_code IN (1, 3)),
    old_binding_id UUID,
    old_binding_version BIGINT CHECK (old_binding_version IS NULL OR old_binding_version > 0),
    old_prior_lifecycle_revision BIGINT CHECK (
        old_prior_lifecycle_revision IS NULL OR old_prior_lifecycle_revision IN (1, 2)
    ),
    old_prior_state SMALLINT CHECK (old_prior_state IS NULL OR old_prior_state IN (1, 2)),
    old_resulting_lifecycle_revision BIGINT CHECK (
        old_resulting_lifecycle_revision IS NULL OR old_resulting_lifecycle_revision IN (1, 2)
    ),
    old_resulting_state SMALLINT CHECK (
        old_resulting_state IS NULL OR old_resulting_state IN (1, 2)
    ),
    successor_binding_id UUID,
    successor_binding_version BIGINT CHECK (
        successor_binding_version IS NULL OR successor_binding_version > 0
    ),
    successor_lifecycle_revision BIGINT CHECK (
        successor_lifecycle_revision IS NULL OR successor_lifecycle_revision = 1
    ),
    successor_state SMALLINT CHECK (successor_state IS NULL OR successor_state = 1),
    operation_id UUID NOT NULL,
    request_fingerprint BYTEA NOT NULL CHECK (octet_length(request_fingerprint) = 32),
    transition_digest BYTEA NOT NULL CHECK (octet_length(transition_digest) = 32),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, history_id),
    UNIQUE (community_id, operation_id),
    UNIQUE (community_id, history_id, operation_id, request_fingerprint),
    UNIQUE (
        community_id,
        history_id,
        successor_binding_id,
        successor_binding_version,
        operation_id,
        request_fingerprint
    ),
    UNIQUE (
        community_id,
        history_id,
        old_binding_id,
        old_binding_version,
        old_resulting_lifecycle_revision,
        old_resulting_state
    ),
    FOREIGN KEY (
        community_id,
        operation_id,
        request_fingerprint,
        transition_kind,
        outcome_code
    ) REFERENCES authorization_operation_receipts (
        community_id,
        operation_id,
        request_fingerprint,
        operation_kind,
        outcome_code
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (community_id, old_binding_id, old_binding_version)
        REFERENCES identity_bindings (community_id, binding_id, binding_version)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (community_id, successor_binding_id, successor_binding_version)
        REFERENCES identity_bindings (community_id, binding_id, binding_version)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (history_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (
        (old_binding_id IS NULL
            AND old_binding_version IS NULL
            AND old_prior_lifecycle_revision IS NULL
            AND old_prior_state IS NULL
            AND old_resulting_lifecycle_revision IS NULL
            AND old_resulting_state IS NULL)
        OR (old_binding_id IS NOT NULL
            AND old_binding_version IS NOT NULL
            AND old_prior_lifecycle_revision IS NOT NULL
            AND old_prior_state IS NOT NULL
            AND old_resulting_lifecycle_revision IS NOT NULL
            AND old_resulting_state IS NOT NULL)
    ),
    CHECK (
        (successor_binding_id IS NULL
            AND successor_binding_version IS NULL
            AND successor_lifecycle_revision IS NULL
            AND successor_state IS NULL)
        OR (successor_binding_id IS NOT NULL
            AND successor_binding_version IS NOT NULL
            AND successor_lifecycle_revision = 1
            AND successor_state = 1)
    ),
    CHECK (
        old_binding_id IS NULL
        OR successor_binding_id IS NULL
        OR old_binding_id <> successor_binding_id
    ),
    CHECK (
        old_binding_version IS NULL
        OR successor_binding_version IS NULL
        OR old_binding_version <> successor_binding_version
    ),
    -- Core lifecycle only ever moves Active/r1 to Retired/r2 for a named old
    -- generation. Extended re-enablement (recover/enable from Retired/r2) is a
    -- later migration's concern.
    CHECK (
        old_binding_id IS NULL
        OR (old_prior_lifecycle_revision = 1
            AND old_prior_state = 1
            AND old_resulting_lifecycle_revision = 2
            AND old_resulting_state = 2)
    ),
    CHECK (
        (outcome_code = 3
            AND old_binding_id IS NULL
            AND successor_binding_id IS NULL)
        OR (outcome_code = 1 AND (
            (transition_kind = 1
                AND old_binding_id IS NULL
                AND successor_binding_id IS NOT NULL)
            OR (transition_kind = 3
                AND old_binding_id IS NOT NULL
                AND successor_binding_id IS NULL)
            OR (transition_kind = 5
                AND successor_binding_id IS NULL)
            OR (transition_kind = 6
                AND old_binding_id IS NOT NULL
                AND successor_binding_id IS NOT NULL)
        ))
    )
);

CREATE INDEX identity_lifecycle_history_old_binding
    ON identity_lifecycle_history (community_id, old_binding_id, old_binding_version, recorded_at);
CREATE INDEX identity_lifecycle_history_successor_binding
    ON identity_lifecycle_history (
        community_id,
        successor_binding_id,
        successor_binding_version,
        recorded_at
    );

-- Circular birth/transition ordering is deliberate and fully deferred. Every
-- generation must commit with its exact birth transition, and a retired row
-- must commit with the exact transition that changed Active/r1 to Retired/r2.
ALTER TABLE identity_bindings
    ADD CONSTRAINT identity_bindings_exact_birth_history_fk
    FOREIGN KEY (
        community_id,
        birth_history_id,
        binding_id,
        binding_version,
        creation_operation_id,
        creation_request_fingerprint
    ) REFERENCES identity_lifecycle_history (
        community_id,
        history_id,
        successor_binding_id,
        successor_binding_version,
        operation_id,
        request_fingerprint
    ) DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE identity_bindings
    ADD CONSTRAINT identity_bindings_exact_retirement_history_fk
    FOREIGN KEY (
        community_id,
        retirement_history_id,
        binding_id,
        binding_version,
        lifecycle_revision,
        binding_state
    ) REFERENCES identity_lifecycle_history (
        community_id,
        history_id,
        old_binding_id,
        old_binding_version,
        old_resulting_lifecycle_revision,
        old_resulting_state
    ) DEFERRABLE INITIALLY DEFERRED;

-- One immutable closed-scope fact table. Core selector kinds only:
-- 1 retired pair (P), 3 revoked key (Y). Both are permanent. The extended
-- disabled-identity (X) and pending-replacement (Q) selectors, and their
-- one-shot consumption, are introduced by the FI-LIFECYCLE migration.
CREATE TABLE identity_lifecycle_selectors (
    community_id UUID NOT NULL REFERENCES communities(id),
    selector_id UUID NOT NULL,
    selector_kind SMALLINT NOT NULL CHECK (selector_kind IN (1, 3)),
    selector_fingerprint BYTEA NOT NULL CHECK (octet_length(selector_fingerprint) = 32),
    fact_generation BIGINT NOT NULL CHECK (fact_generation > 0),
    principal_fingerprint BYTEA CHECK (
        principal_fingerprint IS NULL OR octet_length(principal_fingerprint) = 32
    ),
    event_author_pubkey BYTEA CHECK (
        event_author_pubkey IS NULL OR octet_length(event_author_pubkey) = 32
    ),
    binding_id UUID,
    binding_version BIGINT CHECK (binding_version IS NULL OR binding_version > 0),
    asserted_history_id UUID NOT NULL,
    selected_by_operation_id UUID NOT NULL,
    selected_by_request_fingerprint BYTEA NOT NULL CHECK (
        octet_length(selected_by_request_fingerprint) = 32
    ),
    selected_at TIMESTAMPTZ NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (community_id, selector_id),
    UNIQUE (community_id, selector_id, selector_kind),
    UNIQUE (community_id, selector_kind, selector_fingerprint, fact_generation),
    FOREIGN KEY (
        community_id,
        asserted_history_id,
        selected_by_operation_id,
        selected_by_request_fingerprint
    ) REFERENCES identity_lifecycle_history (
        community_id,
        history_id,
        operation_id,
        request_fingerprint
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (
        community_id,
        selected_by_operation_id,
        selected_by_request_fingerprint
    ) REFERENCES authorization_operation_receipts (
        community_id,
        operation_id,
        request_fingerprint
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (community_id, binding_id, binding_version)
        REFERENCES identity_bindings (community_id, binding_id, binding_version)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (selector_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CHECK (
        (selector_kind = 1
            AND fact_generation = 1
            AND principal_fingerprint IS NOT NULL
            AND event_author_pubkey IS NOT NULL
            AND binding_id IS NOT NULL
            AND binding_version IS NOT NULL)
        OR (selector_kind = 3
            AND fact_generation = 1
            AND principal_fingerprint IS NULL
            AND event_author_pubkey IS NOT NULL
            AND binding_id IS NULL
            AND binding_version IS NULL)
    )
);

CREATE UNIQUE INDEX identity_lifecycle_selectors_permanent_pair
    ON identity_lifecycle_selectors (community_id, binding_id, binding_version)
    WHERE selector_kind = 1;
CREATE UNIQUE INDEX identity_lifecycle_selectors_permanent_principal_key
    ON identity_lifecycle_selectors (
        community_id,
        principal_fingerprint,
        event_author_pubkey
    ) WHERE selector_kind = 1;
CREATE UNIQUE INDEX identity_lifecycle_selectors_permanent_key
    ON identity_lifecycle_selectors (community_id, event_author_pubkey)
    WHERE selector_kind = 3;
CREATE INDEX identity_lifecycle_selectors_principal_lookup
    ON identity_lifecycle_selectors
        (community_id, selector_kind, principal_fingerprint, fact_generation);
CREATE INDEX identity_lifecycle_selectors_key_lookup
    ON identity_lifecycle_selectors
        (community_id, selector_kind, event_author_pubkey, fact_generation);
CREATE INDEX identity_lifecycle_selectors_binding_lookup
    ON identity_lifecycle_selectors
        (community_id, selector_kind, binding_id, binding_version, fact_generation);
CREATE INDEX identity_lifecycle_selectors_asserted_history
    ON identity_lifecycle_selectors
        (community_id, asserted_history_id, selector_kind);

-- Serializes policy-revision inserts per community: each new revision must
-- strictly exceed the current maximum (FI-INV-06 — stable assertion policy
-- anchor; a backfilled or replayed revision is incoherent). The per-community
-- advisory lock prevents two concurrent writers from both passing a plain
-- SELECT MAX() check and committing conflicting revisions.
CREATE FUNCTION identity_enrollment_policy_revision_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    lock_key BIGINT;
    max_revision BIGINT;
BEGIN
    -- Acquire a per-community exclusive transaction-scoped advisory lock so
    -- that concurrent insertions serialize here. The key is a stable hash of
    -- the namespace string and the community_id bytes.
    lock_key := hashtextextended(
        'buzz:enrollment-policy-revision:v1:' || NEW.community_id::text,
        0
    );
    PERFORM pg_advisory_xact_lock(lock_key);

    SELECT MAX(policy_revision)
    INTO max_revision
    FROM identity_enrollment_policies
    WHERE community_id = NEW.community_id;

    IF max_revision IS NOT NULL
        AND NEW.policy_revision <= max_revision
    THEN
        RAISE EXCEPTION
            'policy_revision % does not strictly exceed current maximum % for community %',
            NEW.policy_revision, max_revision, NEW.community_id
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_enrollment_policy_revision_monotonic';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION nip_fi_reject_row_mutation_v1() RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION '% is immutable', TG_TABLE_NAME
        USING ERRCODE = 'check_violation';
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION nip_fi_reject_truncate_v1() RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION '% cannot be truncated', TG_TABLE_NAME
        USING ERRCODE = 'check_violation';
END;
$$ LANGUAGE plpgsql;

-- Every binding/selector path derives the same domain-scoped coordinates and
-- takes their signed BIGINT advisory keys in numeric order. Typed transaction
-- APIs take these locks before row mutation; the triggers are the fail-closed
-- backstop for direct SQL.
CREATE FUNCTION identity_lifecycle_lock_coordinates_v1(
    locked_community_id UUID,
    locked_principal_fingerprint BYTEA,
    locked_event_author_pubkey BYTEA
) RETURNS VOID AS $$
DECLARE
    principal_lock_key BIGINT;
    event_author_lock_key BIGINT;
BEGIN
    IF locked_principal_fingerprint IS NOT NULL THEN
        principal_lock_key := hashtextextended(
            'buzz:identity-lifecycle-coordinate:v1:principal:'
                || locked_community_id::text || ':'
                || encode(locked_principal_fingerprint, 'hex'),
            0
        );
    END IF;
    IF locked_event_author_pubkey IS NOT NULL THEN
        event_author_lock_key := hashtextextended(
            'buzz:identity-lifecycle-coordinate:v1:key:'
                || locked_community_id::text || ':'
                || encode(locked_event_author_pubkey, 'hex'),
            0
        );
    END IF;

    IF principal_lock_key IS NOT NULL AND event_author_lock_key IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(LEAST(principal_lock_key, event_author_lock_key));
        IF principal_lock_key <> event_author_lock_key THEN
            PERFORM pg_advisory_xact_lock(GREATEST(principal_lock_key, event_author_lock_key));
        END IF;
    ELSIF principal_lock_key IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(principal_lock_key);
    ELSIF event_author_lock_key IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(event_author_lock_key);
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_bindings_insert_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    PERFORM identity_lifecycle_lock_coordinates_v1(
        NEW.community_id,
        NEW.principal_fingerprint,
        NEW.event_author_pubkey
    );
    IF NEW.binding_state <> 1
        OR NEW.lifecycle_revision <> 1
        OR NEW.retirement_history_id IS NOT NULL
    THEN
        RAISE EXCEPTION 'identity binding birth must be Active at lifecycle revision 1'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_birth_state';
    END IF;
    NEW.created_at := transaction_timestamp();
    NEW.updated_at := transaction_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_bindings_transition_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    PERFORM identity_lifecycle_lock_coordinates_v1(
        OLD.community_id,
        OLD.principal_fingerprint,
        OLD.event_author_pubkey
    );
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.binding_id IS DISTINCT FROM OLD.binding_id
        OR NEW.binding_version IS DISTINCT FROM OLD.binding_version
        OR NEW.issuer IS DISTINCT FROM OLD.issuer
        OR NEW.subject IS DISTINCT FROM OLD.subject
        OR NEW.principal_fingerprint IS DISTINCT FROM OLD.principal_fingerprint
        OR NEW.event_author_pubkey IS DISTINCT FROM OLD.event_author_pubkey
        OR NEW.binding_provenance IS DISTINCT FROM OLD.binding_provenance
        OR NEW.policy_revision IS DISTINCT FROM OLD.policy_revision
        OR NEW.enrollment_evidence_digest IS DISTINCT FROM OLD.enrollment_evidence_digest
        OR NEW.expires_at IS DISTINCT FROM OLD.expires_at
        OR NEW.birth_history_id IS DISTINCT FROM OLD.birth_history_id
        OR NEW.creation_operation_id IS DISTINCT FROM OLD.creation_operation_id
        OR NEW.creation_request_fingerprint IS DISTINCT FROM OLD.creation_request_fingerprint
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'identity binding generation coordinates are immutable'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_immutable_generation';
    END IF;
    IF OLD.binding_state <> 1
        OR OLD.lifecycle_revision <> 1
        OR OLD.retirement_history_id IS NOT NULL
        OR NEW.binding_state <> 2
        OR NEW.lifecycle_revision <> 2
        OR NEW.retirement_history_id IS NULL
        OR NEW.retirement_history_id = OLD.birth_history_id
    THEN
        RAISE EXCEPTION 'identity binding permits only Active/r1 to Retired/r2'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_active_to_retired';
    END IF;
    NEW.updated_at := transaction_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_lifecycle_history_insert_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    NEW.recorded_at := transaction_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_binding_history_semantics_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    retirement identity_lifecycle_history%ROWTYPE;
BEGIN
    IF NEW.binding_state = 2 THEN
        SELECT * INTO STRICT retirement
        FROM identity_lifecycle_history
        WHERE community_id = NEW.community_id
          AND history_id = NEW.retirement_history_id
          AND old_binding_id = NEW.binding_id
          AND old_binding_version = NEW.binding_version;
        IF retirement.outcome_code <> 1
            OR retirement.old_prior_lifecycle_revision <> 1
            OR retirement.old_prior_state <> 1
            OR retirement.old_resulting_lifecycle_revision <> 2
            OR retirement.old_resulting_state <> 2
        THEN
            RAISE EXCEPTION 'retired binding must reference its exact Active-to-Retired transition'
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'identity_bindings_retirement_history_semantics';
        END IF;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_binding_birth_eligibility_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM identity_lifecycle_selectors selector
        WHERE selector.community_id = NEW.community_id
          AND (
            (selector.selector_kind = 1
                AND selector.principal_fingerprint = NEW.principal_fingerprint
                AND selector.event_author_pubkey = NEW.event_author_pubkey)
            OR (selector.selector_kind = 3
                AND selector.event_author_pubkey = NEW.event_author_pubkey)
          )
    ) THEN
        RAISE EXCEPTION 'binding birth conflicts with an effective lifecycle selector'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_birth_eligibility';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION authorization_operation_receipt_history_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    history_count BIGINT;
    expected_count BIGINT;
BEGIN
    SELECT count(*) INTO history_count
    FROM identity_lifecycle_history history
    WHERE history.community_id = NEW.community_id
      AND history.operation_id = NEW.operation_id;

    -- Core lifecycle receipts (enroll, retire, revoke, rotate) each require
    -- exactly one lifecycle-history row. Non-lifecycle receipts (protected
    -- mutation, invalidation) require none.
    expected_count := CASE
        WHEN NEW.operation_kind IN (1, 3, 5, 6) AND NEW.outcome_code IN (1, 3) THEN 1
        ELSE 0
    END;
    IF history_count <> expected_count THEN
        RAISE EXCEPTION 'operation receipt requires % lifecycle history row, found %',
            expected_count, history_count
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_operation_receipt_history_cardinality';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_lifecycle_selector_insert_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    NEW.selected_at := transaction_timestamp();
    PERFORM identity_lifecycle_lock_coordinates_v1(
        NEW.community_id,
        CASE WHEN NEW.selector_kind = 1 THEN NEW.principal_fingerprint END,
        NEW.event_author_pubkey
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_lifecycle_selector_history_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    history identity_lifecycle_history%ROWTYPE;
    old_binding identity_bindings%ROWTYPE;
BEGIN
    SELECT * INTO STRICT history
    FROM identity_lifecycle_history
    WHERE community_id = NEW.community_id
      AND history_id = NEW.asserted_history_id
      AND operation_id = NEW.selected_by_operation_id
      AND request_fingerprint = NEW.selected_by_request_fingerprint;

    IF history.old_binding_id IS NOT NULL THEN
        SELECT * INTO STRICT old_binding
        FROM identity_bindings
        WHERE community_id = history.community_id
          AND binding_id = history.old_binding_id
          AND binding_version = history.old_binding_version;
    END IF;

    -- A retired-pair (P) selector is asserted by retire, revoke, or rotate of a
    -- named old generation; a revoked-key (Y) selector by revoke.
    IF history.outcome_code <> 1
        OR (NEW.selector_kind = 1 AND (
            history.transition_kind NOT IN (3, 5, 6)
            OR history.old_binding_id IS DISTINCT FROM NEW.binding_id
            OR history.old_binding_version IS DISTINCT FROM NEW.binding_version
            OR old_binding.principal_fingerprint IS DISTINCT FROM NEW.principal_fingerprint
            OR old_binding.event_author_pubkey IS DISTINCT FROM NEW.event_author_pubkey
        ))
        OR (NEW.selector_kind = 3 AND (
            history.transition_kind <> 5
            OR (history.old_binding_id IS NOT NULL
                AND old_binding.event_author_pubkey
                    IS DISTINCT FROM NEW.event_author_pubkey)
        ))
    THEN
        RAISE EXCEPTION 'selector does not match its lifecycle transition'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_selector_history_semantics';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION identity_lifecycle_transition_integrity_guard_v1() RETURNS TRIGGER AS $$
DECLARE
    transition identity_lifecycle_history%ROWTYPE;
    old_binding_state SMALLINT;
    asserted_p BIGINT;
    asserted_y BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'identity_lifecycle_history' THEN
        transition := NEW;
    ELSIF TG_TABLE_NAME = 'identity_lifecycle_selectors' THEN
        SELECT * INTO STRICT transition
        FROM identity_lifecycle_history
        WHERE community_id = NEW.community_id
          AND history_id = NEW.asserted_history_id;
    ELSE
        SELECT * INTO STRICT transition
        FROM identity_lifecycle_history
        WHERE community_id = NEW.community_id
          AND history_id = CASE
              WHEN NEW.binding_state = 2 THEN NEW.retirement_history_id
              ELSE NEW.birth_history_id
          END;
    END IF;

    SELECT
        count(*) FILTER (WHERE selector_kind = 1),
        count(*) FILTER (WHERE selector_kind = 3)
    INTO asserted_p, asserted_y
    FROM identity_lifecycle_selectors
    WHERE community_id = transition.community_id
      AND asserted_history_id = transition.history_id;

    IF transition.old_binding_id IS NOT NULL THEN
        SELECT binding_state INTO STRICT old_binding_state
        FROM identity_bindings
        WHERE community_id = transition.community_id
          AND binding_id = transition.old_binding_id
          AND binding_version = transition.old_binding_version;
        IF old_binding_state <> 2 THEN
            RAISE EXCEPTION 'lifecycle transition old binding must be retired at commit'
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'identity_lifecycle_transition_integrity';
        END IF;
    END IF;

    IF EXISTS (
        SELECT 1
        FROM identity_lifecycle_selectors selector
        JOIN identity_bindings active
          ON active.community_id = selector.community_id
         AND active.binding_state = 1
         AND (
            (selector.selector_kind = 1
                AND active.principal_fingerprint = selector.principal_fingerprint
                AND active.event_author_pubkey = selector.event_author_pubkey)
            OR (selector.selector_kind = 3
                AND active.event_author_pubkey = selector.event_author_pubkey)
         )
        WHERE selector.community_id = transition.community_id
    ) THEN
        RAISE EXCEPTION 'effective lifecycle selector conflicts with an active binding'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_transition_integrity';
    END IF;

    IF transition.outcome_code = 3 THEN
        IF asserted_p + asserted_y <> 0 THEN
            RAISE EXCEPTION 'no-op lifecycle transition cannot create selector facts'
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'identity_lifecycle_transition_integrity';
        END IF;
        RETURN NULL;
    END IF;

    -- Core selector companions per transition:
    --   enroll  (1): none
    --   retire  (3): exactly one P
    --   revoke  (5): one Y always; one P when a named old generation is removed
    --   rotate  (6): exactly one P (old generation retired)
    IF (transition.transition_kind = 1
            AND (asserted_p, asserted_y) <> (0, 0))
        OR (transition.transition_kind = 3
            AND (asserted_p, asserted_y) <> (1, 0))
        OR (transition.transition_kind = 5 AND (
            (transition.old_binding_id IS NOT NULL
                AND (asserted_p, asserted_y) <> (1, 1))
            OR (transition.old_binding_id IS NULL
                AND (asserted_p, asserted_y) <> (0, 1))
        ))
        OR (transition.transition_kind = 6
            AND (asserted_p, asserted_y) <> (1, 0))
    THEN
        RAISE EXCEPTION 'lifecycle transition has incomplete or forbidden selector companions'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_transition_integrity';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER identity_bindings_insert_guard
    BEFORE INSERT ON identity_bindings
    FOR EACH ROW EXECUTE FUNCTION identity_bindings_insert_guard_v1();
CREATE TRIGGER identity_bindings_transition_guard
    BEFORE UPDATE ON identity_bindings
    FOR EACH ROW EXECUTE FUNCTION identity_bindings_transition_guard_v1();
CREATE CONSTRAINT TRIGGER identity_bindings_history_semantics
    AFTER INSERT OR UPDATE ON identity_bindings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_binding_history_semantics_guard_v1();
CREATE CONSTRAINT TRIGGER identity_bindings_birth_eligibility
    AFTER INSERT ON identity_bindings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_binding_birth_eligibility_guard_v1();
CREATE CONSTRAINT TRIGGER identity_bindings_transition_integrity
    AFTER INSERT OR UPDATE ON identity_bindings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_transition_integrity_guard_v1();
CREATE TRIGGER identity_bindings_no_delete
    BEFORE DELETE ON identity_bindings
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER identity_bindings_no_truncate
    BEFORE TRUNCATE ON identity_bindings
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER identity_lifecycle_history_insert_guard
    BEFORE INSERT ON identity_lifecycle_history
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_history_insert_guard_v1();
CREATE CONSTRAINT TRIGGER authorization_operation_receipt_history_cardinality
    AFTER INSERT ON authorization_operation_receipts
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION authorization_operation_receipt_history_guard_v1();
CREATE CONSTRAINT TRIGGER identity_lifecycle_transition_integrity
    AFTER INSERT ON identity_lifecycle_history
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_transition_integrity_guard_v1();

CREATE TRIGGER identity_lifecycle_selector_insert_guard
    BEFORE INSERT ON identity_lifecycle_selectors
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_selector_insert_guard_v1();
CREATE CONSTRAINT TRIGGER identity_lifecycle_selector_history_semantics
    AFTER INSERT ON identity_lifecycle_selectors
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_selector_history_guard_v1();
CREATE CONSTRAINT TRIGGER identity_lifecycle_selector_transition_integrity
    AFTER INSERT ON identity_lifecycle_selectors
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION identity_lifecycle_transition_integrity_guard_v1();

CREATE TRIGGER authorization_operation_receipts_immutable
    BEFORE UPDATE OR DELETE ON authorization_operation_receipts
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_operation_receipts_no_truncate
    BEFORE TRUNCATE ON authorization_operation_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER identity_enrollment_policies_revision_guard
    BEFORE INSERT ON identity_enrollment_policies
    FOR EACH ROW EXECUTE FUNCTION identity_enrollment_policy_revision_guard_v1();
CREATE TRIGGER identity_enrollment_policies_immutable
    BEFORE UPDATE OR DELETE ON identity_enrollment_policies
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER identity_enrollment_policies_no_truncate
    BEFORE TRUNCATE ON identity_enrollment_policies
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER identity_lifecycle_history_immutable
    BEFORE UPDATE OR DELETE ON identity_lifecycle_history
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER identity_lifecycle_history_no_truncate
    BEFORE TRUNCATE ON identity_lifecycle_history
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

CREATE TRIGGER identity_lifecycle_selectors_immutable
    BEFORE UPDATE OR DELETE ON identity_lifecycle_selectors
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER identity_lifecycle_selectors_no_truncate
    BEFORE TRUNCATE ON identity_lifecycle_selectors
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();

-- These identity relations are a durable, tamper-evident authorization ledger:
-- FI-INV-02 (durable binding) and FI-INV-03 (tombstone monotonicity) require
-- their denial facts to outlive any single tenant lifecycle, and the immutable
-- no_delete/no_truncate triggers above enforce exactly that. They therefore
-- carry community_id as provenance, not as deletable ownership — the same
-- posture migration 0030 took for product_feedback and rate_limit_violations.
-- Widen the single SQL source of truth so the universal write fence and the
-- deletion catalog treat them as ledger: never fence-attached, never purged,
-- never counted as tenant-scoped drift. community rows are permanent tombstones
-- (never hard-deleted), so their NOT NULL community_id references never dangle.
CREATE OR REPLACE FUNCTION community_write_fence_excluded_table(target NAME) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT target::TEXT = ANY (ARRAY[
        'community_deletion_requests', 'community_deletion_approvals',
        'community_deletion_checkpoints', 'community_serving_write_leases',
        'community_deletion_executor_heartbeats', 'product_feedback',
        'rate_limit_violations',
        'authorization_operation_receipts', 'identity_enrollment_policies',
        'identity_bindings', 'identity_lifecycle_history',
        'identity_lifecycle_selectors'
    ]::TEXT[])
$$;
