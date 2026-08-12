-- Bounded authorization-audit health and credential-free operator denials.
--
-- Migration 0030 owns the immutable event ledger and its hard ceilings. This
-- upgrade reserves bounded room for fail-closed/security events, adds explicit
-- failure/recovery generations, and records pre-authentication denials in a
-- fixed-cardinality bucket table. It never stores a proof, nonce, assertion,
-- provider identifier, principal, email address, or display value.

ALTER TABLE authorization_event_capacity
    ADD COLUMN restrictive_reserve_events BIGINT,
    ADD COLUMN restrictive_reserve_bytes BIGINT,
    ADD COLUMN failure_generation BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN recovery_generation BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN recovered_at TIMESTAMPTZ;

UPDATE authorization_event_capacity
SET restrictive_reserve_events = CASE
        WHEN max_events_per_domain > 1
            THEN GREATEST(1, LEAST(64, max_events_per_domain / 8))
        ELSE 0
    END,
    restrictive_reserve_bytes = CASE
        WHEN max_bytes_per_domain > max_envelope_bytes
            THEN GREATEST(
                max_envelope_bytes,
                LEAST(262144, max_bytes_per_domain / 8)
            )
        ELSE 0
    END,
    failure_generation = CASE WHEN health_state = 2 THEN 1 ELSE 0 END,
    recovery_generation = 0;

-- Preserve safe compatibility for callers compiled before this migration:
-- omitted reserve columns are derived from the server-owned hard ceilings,
-- while callers cannot select a weaker value because the checks below pin the
-- exact derivation.
CREATE FUNCTION authorization_event_capacity_reserve_defaults_v2() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.restrictive_reserve_events IS NULL THEN
        NEW.restrictive_reserve_events := CASE
            WHEN NEW.max_events_per_domain > 1
                THEN GREATEST(1, LEAST(64, NEW.max_events_per_domain / 8))
            ELSE 0
        END;
    END IF;
    IF NEW.restrictive_reserve_bytes IS NULL THEN
        NEW.restrictive_reserve_bytes := CASE
            WHEN NEW.max_bytes_per_domain > NEW.max_envelope_bytes
                THEN GREATEST(
                    NEW.max_envelope_bytes,
                    LEAST(262144, NEW.max_bytes_per_domain / 8)
                )
            ELSE 0
        END;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_event_capacity_reserve_defaults
    BEFORE INSERT ON authorization_event_capacity
    FOR EACH ROW EXECUTE FUNCTION authorization_event_capacity_reserve_defaults_v2();

ALTER TABLE authorization_event_capacity
    ALTER COLUMN restrictive_reserve_events SET NOT NULL,
    ALTER COLUMN restrictive_reserve_bytes SET NOT NULL,
    ADD CONSTRAINT authorization_event_capacity_reserve_events CHECK (
        restrictive_reserve_events = CASE
            WHEN max_events_per_domain > 1
                THEN GREATEST(1, LEAST(64, max_events_per_domain / 8))
            ELSE 0
        END
    ),
    ADD CONSTRAINT authorization_event_capacity_reserve_bytes CHECK (
        restrictive_reserve_bytes = CASE
            WHEN max_bytes_per_domain > max_envelope_bytes
                THEN GREATEST(
                    max_envelope_bytes,
                    LEAST(262144, max_bytes_per_domain / 8)
                )
            ELSE 0
        END
    ),
    ADD CONSTRAINT authorization_event_capacity_generations CHECK (
        recovery_generation <= failure_generation
    ),
    ADD CONSTRAINT authorization_event_capacity_health_generation CHECK (
        (health_state = 1
            AND failure_code IS NULL
            AND failure_observed_at IS NULL
            AND recovery_generation = failure_generation)
        OR (health_state = 2
            AND failure_code IS NOT NULL
            AND failure_observed_at IS NOT NULL
            AND recovery_generation < failure_generation)
    );

DROP TRIGGER authorization_events_capacity ON authorization_events;
DROP FUNCTION authorization_event_capacity_before_insert_v1();

-- One locked capacity row is the complete O(1) admission decision. General
-- success traffic may not consume the restrictive reserve. Lifecycle loss,
-- denial, invalidation, and other non-success evidence may consume it.
CREATE FUNCTION authorization_event_capacity_before_insert_v2() RETURNS TRIGGER AS $$
DECLARE
    policy authorization_event_capacity%ROWTYPE;
    envelope_bytes BIGINT;
    restrictive_event BOOLEAN;
    event_limit BIGINT;
    byte_limit BIGINT;
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
    restrictive_event := NEW.outcome_code <> 1
        OR NEW.event_kind IN (2, 3, 4, 5, 6, 7, 8, 9, 11, 14);
    IF restrictive_event THEN
        event_limit := policy.max_events_per_domain;
        byte_limit := policy.max_bytes_per_domain;
    ELSE
        event_limit := policy.max_events_per_domain
            - policy.restrictive_reserve_events;
        byte_limit := policy.max_bytes_per_domain
            - policy.restrictive_reserve_bytes;
    END IF;

    IF envelope_bytes > policy.max_envelope_bytes
        OR policy.retained_event_count + 1 > event_limit
        OR policy.retained_envelope_bytes + envelope_bytes > byte_limit
    THEN
        RAISE EXCEPTION 'authorization event capacity exhausted'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_exhausted';
    END IF;

    UPDATE authorization_event_capacity
    SET retained_event_count = retained_event_count + 1,
        retained_envelope_bytes = retained_envelope_bytes + envelope_bytes,
        updated_at = GREATEST(
            transaction_timestamp(),
            authorization_event_capacity.updated_at + INTERVAL '1 microsecond'
        )
    WHERE community_id = NEW.community_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_events_capacity
    BEFORE INSERT ON authorization_events
    FOR EACH ROW EXECUTE FUNCTION authorization_event_capacity_before_insert_v2();

DROP TRIGGER authorization_event_capacity_monotonic ON authorization_event_capacity;
DROP FUNCTION authorization_event_capacity_guard_v1();

-- Ceilings/reserves may only rise within their immutable hard caps; retained
-- counters never fall. Recovery is possible only by matching the current
-- failure generation, so a stale health probe cannot clear a later failure.
CREATE FUNCTION authorization_event_capacity_guard_v2() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.configured_at IS DISTINCT FROM OLD.configured_at
        OR NEW.max_events_per_domain < OLD.max_events_per_domain
        OR NEW.max_bytes_per_domain < OLD.max_bytes_per_domain
        OR NEW.max_envelope_bytes < OLD.max_envelope_bytes
        OR NEW.restrictive_reserve_events < OLD.restrictive_reserve_events
        OR NEW.restrictive_reserve_bytes < OLD.restrictive_reserve_bytes
        OR NEW.retained_event_count < OLD.retained_event_count
        OR NEW.retained_envelope_bytes < OLD.retained_envelope_bytes
        OR NEW.failure_generation < OLD.failure_generation
        OR NEW.recovery_generation < OLD.recovery_generation
        OR NEW.updated_at <= OLD.updated_at
        OR NEW.failure_generation > OLD.failure_generation + 1
        OR NEW.recovery_generation > NEW.failure_generation
        OR (OLD.health_state = 1 AND NEW.health_state = 2 AND (
            NEW.failure_generation <> OLD.failure_generation + 1
            OR NEW.recovery_generation <> OLD.recovery_generation
            OR NEW.failure_code IS NULL
            OR NEW.failure_observed_at IS NULL
            OR NEW.recovered_at IS DISTINCT FROM OLD.recovered_at
        ))
        OR (OLD.health_state = 2 AND NEW.health_state = 1 AND (
            NEW.failure_generation <> OLD.failure_generation
            OR NEW.recovery_generation <> OLD.failure_generation
            OR NEW.failure_code IS NOT NULL
            OR NEW.failure_observed_at IS NOT NULL
            OR NEW.recovered_at IS NULL
            OR NEW.recovered_at <= OLD.failure_observed_at
        ))
        OR (OLD.health_state = NEW.health_state AND (
            NEW.failure_generation <> OLD.failure_generation
            OR NEW.recovery_generation <> OLD.recovery_generation
            OR NEW.failure_code IS DISTINCT FROM OLD.failure_code
            OR NEW.failure_observed_at IS DISTINCT FROM OLD.failure_observed_at
            OR NEW.recovered_at IS DISTINCT FROM OLD.recovered_at
        ))
    THEN
        RAISE EXCEPTION 'authorization event capacity transition is not monotonic'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_monotonic_v2';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_event_capacity_monotonic
    BEFORE UPDATE ON authorization_event_capacity
    FOR EACH ROW EXECUTE FUNCTION authorization_event_capacity_guard_v2();

CREATE FUNCTION authorization_event_capacity_report_failure_v2(
    selected_community_id UUID,
    selected_failure_code SMALLINT
) RETURNS BIGINT AS $$
DECLARE
    next_generation BIGINT;
BEGIN
    IF selected_failure_code NOT IN (1, 2, 3) THEN
        RAISE EXCEPTION 'invalid authorization audit failure code'
            USING ERRCODE = 'check_violation';
    END IF;
    UPDATE authorization_event_capacity
    SET health_state = 2,
        failure_code = selected_failure_code,
        failure_observed_at = transaction_timestamp(),
        failure_generation = failure_generation + 1,
        updated_at = GREATEST(
            transaction_timestamp(),
            authorization_event_capacity.updated_at + INTERVAL '1 microsecond'
        )
    WHERE community_id = selected_community_id AND health_state = 1
    RETURNING failure_generation INTO next_generation;
    IF next_generation IS NULL THEN
        SELECT failure_generation INTO next_generation
        FROM authorization_event_capacity
        WHERE community_id = selected_community_id AND health_state = 2;
    END IF;
    IF next_generation IS NULL THEN
        RAISE EXCEPTION 'authorization event capacity policy missing'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_policy_required';
    END IF;
    RETURN next_generation;
END;
$$ LANGUAGE plpgsql VOLATILE STRICT;

CREATE FUNCTION authorization_event_capacity_recover_v2(
    selected_community_id UUID,
    expected_failure_generation BIGINT
) RETURNS BOOLEAN AS $$
DECLARE
    recovered BOOLEAN;
BEGIN
    UPDATE authorization_event_capacity
    SET health_state = 1,
        failure_code = NULL,
        failure_observed_at = NULL,
        recovery_generation = failure_generation,
        recovered_at = transaction_timestamp(),
        updated_at = GREATEST(
            transaction_timestamp(),
            authorization_event_capacity.updated_at + INTERVAL '1 microsecond'
        )
    WHERE community_id = selected_community_id
      AND health_state = 2
      AND failure_generation = expected_failure_generation;
    recovered := FOUND;
    RETURN recovered;
END;
$$ LANGUAGE plpgsql VOLATILE STRICT;

-- Keep rejected replay distinct from an accepted exact replay and from an
-- operation-identity conflict. Migration 0030 intentionally reserved the
-- original closed reason set; this migration extends the immutable event
-- contract before any protected-denial events can use the new code.
ALTER TABLE authorization_events
    DROP CONSTRAINT authorization_events_reason_code_check;
ALTER TABLE authorization_events
    ADD CONSTRAINT authorization_events_reason_code_check CHECK (
        reason_code IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17)
    );

-- Fixed cardinality: eight operator actions plus one invite action plus one
-- moderation action, each with eight denial classes and twelve rotating
-- five-minute slots = at most 960 rows per server-owned domain. Buckets retain
-- no actor, credential, target, request, or error detail.
CREATE TABLE authorization_operator_denial_buckets (
    community_id UUID NOT NULL REFERENCES communities(id),
    surface_kind SMALLINT NOT NULL CHECK (surface_kind BETWEEN 1 AND 3),
    denial_class SMALLINT NOT NULL CHECK (denial_class BETWEEN 1 AND 8),
    action_kind SMALLINT NOT NULL CHECK (action_kind BETWEEN 1 AND 8),
    slot SMALLINT NOT NULL CHECK (slot BETWEEN 0 AND 11),
    window_generation BIGINT NOT NULL CHECK (window_generation >= 0),
    window_started_at TIMESTAMPTZ NOT NULL,
    denial_count BIGINT NOT NULL CHECK (denial_count > 0),
    lifetime_count BIGINT NOT NULL CHECK (lifetime_count > 0),
    last_denied_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, surface_kind, denial_class, action_kind, slot),
    CHECK (
        (surface_kind = 1 AND action_kind BETWEEN 1 AND 8)
        OR (surface_kind = 2 AND action_kind = 1)
        OR (surface_kind = 3 AND action_kind = 2)
    ),
    CHECK (window_started_at <= last_denied_at)
);

CREATE FUNCTION authorization_denial_bucket_record_v1(
    selected_community_id UUID,
    selected_surface_kind SMALLINT,
    selected_denial_class SMALLINT,
    selected_action_kind SMALLINT
) RETURNS BIGINT AS $$
DECLARE
    authoritative_now TIMESTAMPTZ := clock_timestamp();
    generation BIGINT;
    selected_slot SMALLINT;
    retained_count BIGINT;
BEGIN
    IF selected_surface_kind NOT BETWEEN 1 AND 3
        OR selected_denial_class NOT BETWEEN 1 AND 8
        OR selected_action_kind NOT BETWEEN 1 AND 8
        OR (selected_surface_kind = 2 AND selected_action_kind <> 1)
        OR (selected_surface_kind = 3 AND selected_action_kind <> 2)
    THEN
        RAISE EXCEPTION 'invalid denial bucket coordinate'
            USING ERRCODE = 'check_violation';
    END IF;
    generation := floor(extract(epoch FROM authoritative_now) / 300)::BIGINT;
    selected_slot := (generation % 12)::SMALLINT;
    INSERT INTO authorization_operator_denial_buckets (
        community_id, surface_kind, denial_class, action_kind, slot,
        window_generation, window_started_at, denial_count, lifetime_count,
        last_denied_at
    ) VALUES (
        selected_community_id, selected_surface_kind, selected_denial_class,
        selected_action_kind, selected_slot, generation,
        to_timestamp(generation * 300), 1, 1, authoritative_now
    )
    ON CONFLICT (
        community_id, surface_kind, denial_class, action_kind, slot
    ) DO UPDATE SET
        window_generation = GREATEST(
            authorization_operator_denial_buckets.window_generation,
            EXCLUDED.window_generation
        ),
        window_started_at = CASE
            WHEN authorization_operator_denial_buckets.window_generation
                <= EXCLUDED.window_generation
            THEN EXCLUDED.window_started_at
            ELSE authorization_operator_denial_buckets.window_started_at
        END,
        denial_count = CASE
            WHEN authorization_operator_denial_buckets.window_generation
                = EXCLUDED.window_generation
            THEN CASE
                WHEN authorization_operator_denial_buckets.denial_count
                    = 9223372036854775807
                THEN 9223372036854775807
                ELSE authorization_operator_denial_buckets.denial_count + 1
            END
            WHEN authorization_operator_denial_buckets.window_generation
                < EXCLUDED.window_generation
            THEN 1
            WHEN authorization_operator_denial_buckets.denial_count
                = 9223372036854775807
            THEN 9223372036854775807
            ELSE authorization_operator_denial_buckets.denial_count + 1
        END,
        lifetime_count = CASE
            WHEN authorization_operator_denial_buckets.lifetime_count
                = 9223372036854775807
            THEN 9223372036854775807
            ELSE authorization_operator_denial_buckets.lifetime_count + 1
        END,
        last_denied_at = GREATEST(
            authorization_operator_denial_buckets.last_denied_at,
            EXCLUDED.last_denied_at
        )
    RETURNING denial_count INTO retained_count;
    RETURN retained_count;
END;
$$ LANGUAGE plpgsql VOLATILE STRICT;

CREATE FUNCTION authorization_operator_denial_bucket_record_v1(
    selected_community_id UUID,
    selected_denial_class SMALLINT,
    selected_action_kind SMALLINT
) RETURNS BIGINT AS $$
    SELECT authorization_denial_bucket_record_v1(
        selected_community_id, 1::SMALLINT,
        selected_denial_class, selected_action_kind
    );
$$ LANGUAGE sql VOLATILE STRICT;

CREATE FUNCTION authorization_operator_denial_bucket_guard_v1() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.surface_kind IS DISTINCT FROM OLD.surface_kind
        OR NEW.denial_class IS DISTINCT FROM OLD.denial_class
        OR NEW.action_kind IS DISTINCT FROM OLD.action_kind
        OR NEW.slot IS DISTINCT FROM OLD.slot
        OR NEW.window_generation < OLD.window_generation
        OR NEW.lifetime_count IS DISTINCT FROM (CASE
            WHEN OLD.lifetime_count = 9223372036854775807
            THEN 9223372036854775807
            ELSE OLD.lifetime_count + 1
        END)
        OR (NEW.window_generation = OLD.window_generation AND (
            NEW.window_started_at IS DISTINCT FROM OLD.window_started_at
            OR NEW.denial_count IS DISTINCT FROM (CASE
                WHEN OLD.denial_count = 9223372036854775807
                THEN 9223372036854775807
                ELSE OLD.denial_count + 1
            END)
            OR NEW.last_denied_at < OLD.last_denied_at
        ))
        OR (NEW.window_generation > OLD.window_generation AND (
            NEW.denial_count <> 1
            OR NEW.window_started_at <= OLD.window_started_at
        ))
    THEN
        RAISE EXCEPTION 'operator denial bucket transition is invalid'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER authorization_operator_denial_buckets_monotonic
    BEFORE UPDATE ON authorization_operator_denial_buckets
    FOR EACH ROW EXECUTE FUNCTION authorization_operator_denial_bucket_guard_v1();
CREATE TRIGGER authorization_operator_denial_buckets_no_delete
    BEFORE DELETE ON authorization_operator_denial_buckets
    FOR EACH ROW EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();
CREATE TRIGGER authorization_operator_denial_buckets_no_truncate
    BEFORE TRUNCATE ON authorization_operator_denial_buckets
    FOR EACH STATEMENT EXECUTE FUNCTION nip_fi_reject_truncate_v1();
