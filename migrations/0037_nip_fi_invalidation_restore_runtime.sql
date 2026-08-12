-- Provider-free invalidation/restore runtime and desired-catalog closure.
--
-- Migration 0030 directly installs the invalidation domains, floors, and
-- exact operation-version manifests consumed by the authorization runtime.
-- This migration fails closed if that authority is missing or if durable
-- kind-24244 status history reappears. It then converts the publication
-- allocator to an identity-backed sequence without changing the
-- target-serialized allocation contract.

-- Keep database admission policy aligned with the public Rust contract. The
-- previous checks were intentionally stricter while the audit envelope was
-- still being bounded; replacing them is monotonic because every existing
-- policy already satisfies these expanded limits.
ALTER TABLE authorization_event_capacity
    DROP CONSTRAINT authorization_event_capacity_max_events,
    DROP CONSTRAINT authorization_event_capacity_max_bytes,
    DROP CONSTRAINT authorization_event_capacity_max_envelope,
    ADD CONSTRAINT authorization_event_capacity_max_events
        CHECK (max_events_per_domain BETWEEN 1 AND 1000000) NOT VALID,
    ADD CONSTRAINT authorization_event_capacity_max_bytes
        CHECK (max_bytes_per_domain BETWEEN 1 AND 4294967296) NOT VALID,
    ADD CONSTRAINT authorization_event_capacity_max_envelope
        CHECK (max_envelope_bytes BETWEEN 1 AND 65536) NOT VALID;
ALTER TABLE authorization_event_capacity
    VALIDATE CONSTRAINT authorization_event_capacity_max_events,
    VALIDATE CONSTRAINT authorization_event_capacity_max_bytes,
    VALIDATE CONSTRAINT authorization_event_capacity_max_envelope;

DO $$
DECLARE
    old_sequence_value BIGINT;
    old_sequence_called BOOLEAN;
    allocated_floor BIGINT;
    identity_sequence TEXT;
    existing_identity "char";
    event_primary_key_definition TEXT;
    event_primary_key_oid OID;
    event_primary_key_count BIGINT;
BEGIN
    IF to_regclass('public.authorization_invalidation_domains') IS NULL
        OR to_regclass('public.authorization_invalidation_floors') IS NULL
        OR to_regclass('public.authorization_operation_version_delta_manifests') IS NULL
        OR to_regclass('public.authorization_operation_version_deltas') IS NULL
    THEN
        RAISE EXCEPTION 'authorization invalidation/restore authority is incomplete'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'nip_fi_0037_runtime_prerequisites';
    END IF;
    IF to_regclass('public.client_status_revisions') IS NOT NULL THEN
        RAISE EXCEPTION 'kind 24244 status history must remain connection-local'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'nip_fi_0037_ephemeral_status_required';
    END IF;

    SELECT pg_get_constraintdef(oid, true)
    INTO STRICT event_primary_key_definition
    FROM pg_constraint
    WHERE conrelid = 'events'::regclass
      AND conname = 'events_pkey'
      AND contype = 'p';
    IF event_primary_key_definition =
        'PRIMARY KEY (community_id, created_at, id)'
    THEN
        -- The schema catalog normalizes this partitioned key to put the range
        -- key first. The column set and uniqueness are unchanged; tenant/id
        -- reads remain covered by idx_events_community_id.
        EXECUTE 'ALTER TABLE events DROP CONSTRAINT events_pkey';
        EXECUTE 'ALTER TABLE events ADD CONSTRAINT events_pkey '
            'PRIMARY KEY (created_at, community_id, id)';
    ELSIF event_primary_key_definition <>
        'PRIMARY KEY (created_at, community_id, id)'
    THEN
        RAISE EXCEPTION 'events primary key has unexpected definition: %',
            event_primary_key_definition
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'nip_fi_0037_events_primary_key';
    END IF;

    SELECT oid INTO STRICT event_primary_key_oid
    FROM pg_constraint
    WHERE conrelid = 'events'::regclass
      AND conname = 'events_pkey'
      AND contype = 'p';
    SELECT count(*) INTO event_primary_key_count
    FROM pg_constraint
    WHERE (oid = event_primary_key_oid OR conparentid = event_primary_key_oid)
      AND pg_get_constraintdef(oid, true) =
          'PRIMARY KEY (created_at, community_id, id)';
    IF event_primary_key_count <> 9 THEN
        RAISE EXCEPTION 'events primary key did not propagate to all partitions'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'nip_fi_0037_events_primary_key';
    END IF;

    LOCK TABLE protected_publication_projection_outbox IN ACCESS EXCLUSIVE MODE;

    SELECT attidentity INTO STRICT existing_identity
    FROM pg_attribute
    WHERE attrelid = 'protected_publication_projection_outbox'::regclass
      AND attname = 'publication_sequence'
      AND NOT attisdropped;
    IF existing_identity <> '' THEN
        RAISE EXCEPTION 'publication sequence is already identity-backed'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'nip_fi_0037_allocator_precondition';
    END IF;
    IF pg_get_serial_sequence(
        'protected_publication_projection_outbox',
        'publication_sequence'
    ) IS DISTINCT FROM
        'public.protected_publication_projection_sequence_v1'
    THEN
        RAISE EXCEPTION 'publication sequence has unexpected allocator ownership'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'nip_fi_0037_allocator_precondition';
    END IF;

    SELECT last_value, is_called
    INTO STRICT old_sequence_value, old_sequence_called
    FROM protected_publication_projection_sequence_v1;
    SELECT GREATEST(
        COALESCE(max(publication_sequence), 0),
        CASE WHEN old_sequence_called THEN old_sequence_value ELSE 0 END
    )
    INTO allocated_floor
    FROM protected_publication_projection_outbox;

    EXECUTE 'ALTER SEQUENCE protected_publication_projection_sequence_v1 OWNED BY NONE';
    EXECUTE 'DROP SEQUENCE protected_publication_projection_sequence_v1';
    EXECUTE 'ALTER TABLE protected_publication_projection_outbox '
        'ALTER COLUMN publication_sequence ADD GENERATED ALWAYS AS IDENTITY';

    identity_sequence := pg_get_serial_sequence(
        'protected_publication_projection_outbox',
        'publication_sequence'
    );
    IF identity_sequence IS NULL THEN
        RAISE EXCEPTION 'identity publication allocator was not created'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'nip_fi_0037_allocator_identity';
    END IF;
    PERFORM setval(
        identity_sequence::regclass,
        GREATEST(allocated_floor, 1),
        allocated_floor > 0
    );
END;
$$;

-- PostgreSQL fills an identity default before BEFORE INSERT triggers. The
-- preliminary value is intentionally discarded: the final value is consumed
-- only after the exact protected target lock, preserving commit/abort ordering
-- while allowing the desired schema to reproduce sequence ownership. These
-- values are ordering tokens, not contiguous public identifiers, so aborts
-- and the discarded preliminary allocation may leave harmless gaps.
CREATE OR REPLACE FUNCTION protected_publication_projection_insert_guard_v1()
RETURNS TRIGGER AS $$
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
    PERFORM pg_advisory_xact_lock(
        hashtextextended(
            jsonb_build_array(
                'buzz:nip-fi:protected-publication-projection:v1',
                NEW.community_id::TEXT,
                NEW.object_kind,
                encode(NEW.object_key, 'hex'),
                NEW.projection_kind,
                NEW.projection_key
            )::TEXT,
            0
        )
    );
    NEW.publication_sequence := nextval(
        pg_get_serial_sequence(
            'protected_publication_projection_outbox',
            'publication_sequence'
        )::regclass
    );
    NEW.created_at := transaction_timestamp();
    NEW.next_attempt_at := transaction_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
