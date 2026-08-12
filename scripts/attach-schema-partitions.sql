-- Attach partition child tables after pgschema apply.
--
-- pgschema currently emits existing partition children as standalone CREATE TABLE
-- statements when applying schema/schema.sql in CI. The tables exist, but they
-- are not attached to their partitioned parents, so inserts into events or
-- delivery_log fail with "no partition of relation ... found for row". Keep this
-- idempotent: raw psql/schema.sql already attaches these partitions, while
-- pgschema-created schemas need this repair step.

CREATE OR REPLACE FUNCTION pg_temp.drop_cloned_parent_triggers(
    parent_table regclass,
    child_table regclass
) RETURNS void
LANGUAGE plpgsql
AS $$
DECLARE
    cloned_trigger record;
BEGIN
    -- pgschema creates standalone children with copies of every non-internal
    -- parent trigger. PostgreSQL recreates the inherited trigger during
    -- ATTACH, so remove every same-named clone based on the live parent
    -- catalog instead of maintaining a brittle trigger-name allow-list.
    FOR cloned_trigger IN
        SELECT child_trigger.tgname
        FROM pg_trigger AS child_trigger
        JOIN pg_trigger AS parent_trigger
          ON parent_trigger.tgrelid = parent_table
         AND parent_trigger.tgname = child_trigger.tgname
         AND NOT parent_trigger.tgisinternal
        WHERE child_trigger.tgrelid = child_table
          AND NOT child_trigger.tgisinternal
    LOOP
        EXECUTE format(
            'DROP TRIGGER IF EXISTS %I ON %s',
            cloned_trigger.tgname,
            child_table
        );
    END LOOP;
END;
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'events'::regclass
          AND inhrelid = 'events_p_past'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('events', 'events_p_past');
        ALTER TABLE events ATTACH PARTITION events_p_past
            FOR VALUES FROM (MINVALUE) TO ('2026-01-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'events'::regclass
          AND inhrelid = 'events_p2026_01'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('events', 'events_p2026_01');
        ALTER TABLE events ATTACH PARTITION events_p2026_01
            FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'events'::regclass
          AND inhrelid = 'events_p2026_02'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('events', 'events_p2026_02');
        ALTER TABLE events ATTACH PARTITION events_p2026_02
            FOR VALUES FROM ('2026-02-01') TO ('2026-03-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'events'::regclass
          AND inhrelid = 'events_p2026_03'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('events', 'events_p2026_03');
        ALTER TABLE events ATTACH PARTITION events_p2026_03
            FOR VALUES FROM ('2026-03-01') TO ('2026-04-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'events'::regclass
          AND inhrelid = 'events_p2026_04'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('events', 'events_p2026_04');
        ALTER TABLE events ATTACH PARTITION events_p2026_04
            FOR VALUES FROM ('2026-04-01') TO ('2026-05-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'events'::regclass
          AND inhrelid = 'events_p2026_05'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('events', 'events_p2026_05');
        ALTER TABLE events ATTACH PARTITION events_p2026_05
            FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'events'::regclass
          AND inhrelid = 'events_p2026_06'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('events', 'events_p2026_06');
        ALTER TABLE events ATTACH PARTITION events_p2026_06
            FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'events'::regclass
          AND inhrelid = 'events_p_future'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('events', 'events_p_future');
        ALTER TABLE events ATTACH PARTITION events_p_future
            FOR VALUES FROM ('2026-07-01') TO (MAXVALUE);
    END IF;

    -- When pgschema creates partition children as standalone tables, it also
    -- preserves the parent's identity column on delivery_log children. PostgreSQL
    -- rejects attaching a child table that has its own identity column, so each
    -- delivery_log attach path drops that standalone identity first. Raw
    -- schema-created partitions are already attached, so these branches do not
    -- run against inherited partition columns.

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'delivery_log'::regclass
          AND inhrelid = 'delivery_log_p_past'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('delivery_log', 'delivery_log_p_past');
        ALTER TABLE delivery_log_p_past ALTER COLUMN id DROP IDENTITY IF EXISTS;
        ALTER TABLE delivery_log ATTACH PARTITION delivery_log_p_past
            FOR VALUES FROM (MINVALUE) TO ('2026-03-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'delivery_log'::regclass
          AND inhrelid = 'delivery_log_p2026_03'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('delivery_log', 'delivery_log_p2026_03');
        ALTER TABLE delivery_log_p2026_03 ALTER COLUMN id DROP IDENTITY IF EXISTS;
        ALTER TABLE delivery_log ATTACH PARTITION delivery_log_p2026_03
            FOR VALUES FROM ('2026-03-01') TO ('2026-04-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'delivery_log'::regclass
          AND inhrelid = 'delivery_log_p2026_04'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('delivery_log', 'delivery_log_p2026_04');
        ALTER TABLE delivery_log_p2026_04 ALTER COLUMN id DROP IDENTITY IF EXISTS;
        ALTER TABLE delivery_log ATTACH PARTITION delivery_log_p2026_04
            FOR VALUES FROM ('2026-04-01') TO ('2026-05-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'delivery_log'::regclass
          AND inhrelid = 'delivery_log_p2026_05'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('delivery_log', 'delivery_log_p2026_05');
        ALTER TABLE delivery_log_p2026_05 ALTER COLUMN id DROP IDENTITY IF EXISTS;
        ALTER TABLE delivery_log ATTACH PARTITION delivery_log_p2026_05
            FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'delivery_log'::regclass
          AND inhrelid = 'delivery_log_p2026_06'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('delivery_log', 'delivery_log_p2026_06');
        ALTER TABLE delivery_log_p2026_06 ALTER COLUMN id DROP IDENTITY IF EXISTS;
        ALTER TABLE delivery_log ATTACH PARTITION delivery_log_p2026_06
            FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_inherits
        WHERE inhparent = 'delivery_log'::regclass
          AND inhrelid = 'delivery_log_p_future'::regclass
    ) THEN
        PERFORM pg_temp.drop_cloned_parent_triggers('delivery_log', 'delivery_log_p_future');
        ALTER TABLE delivery_log_p_future ALTER COLUMN id DROP IDENTITY IF EXISTS;
        ALTER TABLE delivery_log ATTACH PARTITION delivery_log_p_future
            FOR VALUES FROM ('2026-07-01') TO (MAXVALUE);
    END IF;
END $$;

-- pgschema 1.7.4 omits disjunctive CHECK constraints from its desired-state
-- dump. Close only the retained direct-final NIP-FI catalog gap here. Existing
-- constraints are never dropped or replaced: a same-name definition mismatch
-- aborts the apply so an unsafe desired catalog cannot be accepted silently.
CREATE OR REPLACE FUNCTION pg_temp.ensure_exact_check_constraint(
    constrained_table regclass,
    constraint_name text,
    expected_definition text,
    constraint_expression text
) RETURNS void
LANGUAGE plpgsql
AS $$
DECLARE
    actual_definition text;
BEGIN
    SELECT pg_get_constraintdef(constraint_oid.oid, true)
    INTO actual_definition
    FROM pg_constraint AS constraint_oid
    WHERE constraint_oid.conrelid = constrained_table
      AND constraint_oid.conname = constraint_name;

    IF FOUND AND actual_definition IS DISTINCT FROM expected_definition THEN
        RAISE EXCEPTION 'constraint % on % has unexpected definition: %',
            constraint_name, constrained_table, actual_definition
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = constraint_name;
    END IF;

    IF NOT FOUND THEN
        EXECUTE format(
            'ALTER TABLE %s ADD CONSTRAINT %I CHECK (%s)',
            constrained_table,
            constraint_name,
            constraint_expression
        );

        SELECT pg_get_constraintdef(constraint_oid.oid, true)
        INTO STRICT actual_definition
        FROM pg_constraint AS constraint_oid
        WHERE constraint_oid.conrelid = constrained_table
          AND constraint_oid.conname = constraint_name;

        IF actual_definition IS DISTINCT FROM expected_definition THEN
            RAISE EXCEPTION 'installed constraint % on % has unexpected definition: %',
                constraint_name, constrained_table, actual_definition
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = constraint_name;
        END IF;
    END IF;
END;
$$;

SELECT pg_temp.ensure_exact_check_constraint(
    'authorization_invalidation_floors',
    'authorization_invalidation_floors_check',
    $definition$CHECK (selector_kind = 3 AND binding_version_floor IS NOT NULL AND relationship_revision_floor IS NULL OR selector_kind = 7 AND binding_version_floor IS NULL AND relationship_revision_floor IS NOT NULL OR (selector_kind <> ALL (ARRAY[3, 7])) AND binding_version_floor IS NULL AND relationship_revision_floor IS NULL)$definition$,
    $expression$selector_kind = 3 AND binding_version_floor IS NOT NULL AND relationship_revision_floor IS NULL OR selector_kind = 7 AND binding_version_floor IS NULL AND relationship_revision_floor IS NOT NULL OR (selector_kind <> ALL (ARRAY[3, 7])) AND binding_version_floor IS NULL AND relationship_revision_floor IS NULL$expression$
);

SELECT pg_temp.ensure_exact_check_constraint(
    'identity_bindings',
    'identity_bindings_check1',
    $definition$CHECK (binding_state = 1 AND lifecycle_revision = 1 AND retirement_history_id IS NULL OR binding_state = 2 AND lifecycle_revision = 2 AND retirement_history_id IS NOT NULL)$definition$,
    $expression$binding_state = 1 AND lifecycle_revision = 1 AND retirement_history_id IS NULL OR binding_state = 2 AND lifecycle_revision = 2 AND retirement_history_id IS NOT NULL$expression$
);

SELECT pg_temp.ensure_exact_check_constraint(
    'identity_lifecycle_history',
    'identity_lifecycle_history_check',
    $definition$CHECK (old_binding_id IS NULL AND old_binding_version IS NULL AND old_prior_lifecycle_revision IS NULL AND old_prior_state IS NULL AND old_resulting_lifecycle_revision IS NULL AND old_resulting_state IS NULL OR old_binding_id IS NOT NULL AND old_binding_version IS NOT NULL AND old_prior_lifecycle_revision IS NOT NULL AND old_prior_state IS NOT NULL AND old_resulting_lifecycle_revision IS NOT NULL AND old_resulting_state IS NOT NULL)$definition$,
    $expression$old_binding_id IS NULL AND old_binding_version IS NULL AND old_prior_lifecycle_revision IS NULL AND old_prior_state IS NULL AND old_resulting_lifecycle_revision IS NULL AND old_resulting_state IS NULL OR old_binding_id IS NOT NULL AND old_binding_version IS NOT NULL AND old_prior_lifecycle_revision IS NOT NULL AND old_prior_state IS NOT NULL AND old_resulting_lifecycle_revision IS NOT NULL AND old_resulting_state IS NOT NULL$expression$
);

SELECT pg_temp.ensure_exact_check_constraint(
    'identity_lifecycle_history',
    'identity_lifecycle_history_check1',
    $definition$CHECK (successor_binding_id IS NULL AND successor_binding_version IS NULL AND successor_lifecycle_revision IS NULL AND successor_state IS NULL OR successor_binding_id IS NOT NULL AND successor_binding_version IS NOT NULL AND successor_lifecycle_revision = 1 AND successor_state = 1)$definition$,
    $expression$successor_binding_id IS NULL AND successor_binding_version IS NULL AND successor_lifecycle_revision IS NULL AND successor_state IS NULL OR successor_binding_id IS NOT NULL AND successor_binding_version IS NOT NULL AND successor_lifecycle_revision = 1 AND successor_state = 1$expression$
);

SELECT pg_temp.ensure_exact_check_constraint(
    'identity_lifecycle_history',
    'identity_lifecycle_history_check5',
    $definition$CHECK (outcome_code = 3 AND old_binding_id IS NULL AND successor_binding_id IS NULL OR outcome_code = 1 AND ((transition_kind = ANY (ARRAY[1, 2])) AND old_binding_id IS NULL AND successor_binding_id IS NOT NULL OR transition_kind = 3 AND old_binding_id IS NOT NULL AND successor_binding_id IS NULL OR (transition_kind = ANY (ARRAY[4, 5])) AND successor_binding_id IS NULL OR transition_kind = 6 AND old_binding_id IS NOT NULL AND successor_binding_id IS NOT NULL OR transition_kind = 7 AND old_binding_id IS NOT NULL AND successor_binding_id IS NOT NULL OR transition_kind = 8 AND successor_binding_id IS NOT NULL OR transition_kind = 9 AND old_binding_id IS NOT NULL AND successor_binding_id IS NULL))$definition$,
    $expression$outcome_code = 3 AND old_binding_id IS NULL AND successor_binding_id IS NULL OR outcome_code = 1 AND ((transition_kind = ANY (ARRAY[1, 2])) AND old_binding_id IS NULL AND successor_binding_id IS NOT NULL OR transition_kind = 3 AND old_binding_id IS NOT NULL AND successor_binding_id IS NULL OR (transition_kind = ANY (ARRAY[4, 5])) AND successor_binding_id IS NULL OR transition_kind = 6 AND old_binding_id IS NOT NULL AND successor_binding_id IS NOT NULL OR transition_kind = 7 AND old_binding_id IS NOT NULL AND successor_binding_id IS NOT NULL OR transition_kind = 8 AND successor_binding_id IS NOT NULL OR transition_kind = 9 AND old_binding_id IS NOT NULL AND successor_binding_id IS NULL)$expression$
);

SELECT pg_temp.ensure_exact_check_constraint(
    'identity_lifecycle_selectors',
    'identity_lifecycle_selectors_check',
    $definition$CHECK (selector_kind = 1 AND fact_generation = 1 AND principal_fingerprint IS NOT NULL AND event_author_pubkey IS NOT NULL AND binding_id IS NOT NULL AND binding_version IS NOT NULL OR selector_kind = 2 AND principal_fingerprint IS NOT NULL AND event_author_pubkey IS NULL AND binding_id IS NULL AND binding_version IS NULL OR selector_kind = 3 AND fact_generation = 1 AND principal_fingerprint IS NULL AND event_author_pubkey IS NOT NULL AND binding_id IS NULL AND binding_version IS NULL OR selector_kind = 4 AND principal_fingerprint IS NOT NULL AND event_author_pubkey IS NOT NULL AND binding_id IS NOT NULL AND binding_version IS NOT NULL)$definition$,
    $expression$selector_kind = 1 AND fact_generation = 1 AND principal_fingerprint IS NOT NULL AND event_author_pubkey IS NOT NULL AND binding_id IS NOT NULL AND binding_version IS NOT NULL OR selector_kind = 2 AND principal_fingerprint IS NOT NULL AND event_author_pubkey IS NULL AND binding_id IS NULL AND binding_version IS NULL OR selector_kind = 3 AND fact_generation = 1 AND principal_fingerprint IS NULL AND event_author_pubkey IS NOT NULL AND binding_id IS NULL AND binding_version IS NULL OR selector_kind = 4 AND principal_fingerprint IS NOT NULL AND event_author_pubkey IS NOT NULL AND binding_id IS NOT NULL AND binding_version IS NOT NULL$expression$
);

SELECT pg_temp.ensure_exact_check_constraint(
    'authorization_event_capacity',
    'authorization_event_capacity_check3',
    $definition$CHECK (health_state = 1 AND failure_code IS NULL AND failure_observed_at IS NULL OR health_state = 2 AND failure_code IS NOT NULL AND failure_observed_at IS NOT NULL)$definition$,
    $expression$health_state = 1 AND failure_code IS NULL AND failure_observed_at IS NULL OR health_state = 2 AND failure_code IS NOT NULL AND failure_observed_at IS NOT NULL$expression$
);

SELECT pg_temp.ensure_exact_check_constraint(
    'authorization_events',
    'authorization_events_check',
    $definition$CHECK (actor_kind = 4 AND event_kind = 9 AND request_fingerprint IS NULL OR (actor_kind = ANY (ARRAY[1, 2, 3])) AND request_fingerprint IS NOT NULL)$definition$,
    $expression$actor_kind = 4 AND event_kind = 9 AND request_fingerprint IS NULL OR (actor_kind = ANY (ARRAY[1, 2, 3])) AND request_fingerprint IS NOT NULL$expression$
);

SELECT pg_temp.ensure_exact_check_constraint(
    'authorization_events',
    'authorization_events_check1',
    $definition$CHECK (actor_kind = 4 AND actor_fingerprint IS NULL AND subject_fingerprint IS NULL OR (actor_kind = ANY (ARRAY[1, 2, 3])) AND actor_fingerprint IS NOT NULL)$definition$,
    $expression$actor_kind = 4 AND actor_fingerprint IS NULL AND subject_fingerprint IS NULL OR (actor_kind = ANY (ARRAY[1, 2, 3])) AND actor_fingerprint IS NOT NULL$expression$
);

SELECT pg_temp.ensure_exact_check_constraint(
    'protected_object_authority',
    'protected_object_authority_check1',
    $definition$CHECK (owner_pubkey IS NULL AND delegated_relationship_id IS NULL AND delegated_relationship_revision IS NULL AND delegation_conditions_fingerprint IS NULL OR owner_pubkey IS NOT NULL AND delegated_relationship_id IS NOT NULL AND delegated_relationship_revision IS NOT NULL AND delegation_conditions_fingerprint IS NOT NULL)$definition$,
    $expression$owner_pubkey IS NULL AND delegated_relationship_id IS NULL AND delegated_relationship_revision IS NULL AND delegation_conditions_fingerprint IS NULL OR owner_pubkey IS NOT NULL AND delegated_relationship_id IS NOT NULL AND delegated_relationship_revision IS NOT NULL AND delegation_conditions_fingerprint IS NOT NULL$expression$
);

-- These two disjunctive checks predate the protected authorization schema but
-- are omitted by the pgschema dump path, so this attachment step restores
-- their exact definitions after declarative schema application.
SELECT pg_temp.ensure_exact_check_constraint(
    'moderation_reports',
    'moderation_reports_check',
    $definition$CHECK (target_kind = 'event'::text AND target_event_id IS NOT NULL AND target_pubkey IS NULL AND target_blob_sha256 IS NULL OR target_kind = 'pubkey'::text AND target_event_id IS NULL AND target_pubkey IS NOT NULL AND target_blob_sha256 IS NULL OR target_kind = 'blob'::text AND target_event_id IS NULL AND target_pubkey IS NULL AND target_blob_sha256 IS NOT NULL)$definition$,
    $expression$target_kind = 'event'::text AND target_event_id IS NOT NULL AND target_pubkey IS NULL AND target_blob_sha256 IS NULL OR target_kind = 'pubkey'::text AND target_event_id IS NULL AND target_pubkey IS NOT NULL AND target_blob_sha256 IS NULL OR target_kind = 'blob'::text AND target_event_id IS NULL AND target_pubkey IS NULL AND target_blob_sha256 IS NOT NULL$expression$
);

SELECT pg_temp.ensure_exact_check_constraint(
    'push_leases',
    'push_leases_check',
    $definition$CHECK (active AND app_profile IS NOT NULL AND endpoint_hash IS NOT NULL AND endpoint_grant IS NOT NULL AND max_class IS NOT NULL AND subscriptions IS NOT NULL OR NOT active AND app_profile IS NULL AND endpoint_hash IS NULL AND endpoint_grant IS NULL AND max_class IS NULL AND subscriptions IS NULL)$definition$,
    $expression$active AND app_profile IS NOT NULL AND endpoint_hash IS NOT NULL AND endpoint_grant IS NOT NULL AND max_class IS NOT NULL AND subscriptions IS NOT NULL OR NOT active AND app_profile IS NULL AND endpoint_hash IS NULL AND endpoint_grant IS NULL AND max_class IS NULL AND subscriptions IS NULL$expression$
);
