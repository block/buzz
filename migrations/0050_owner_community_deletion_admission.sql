-- Structured provenance for owner-origin whole-community deletion requests.
--
-- The existing request UUID is the stable correlation/idempotency identity.
-- Owner admission supplies that UUID instead of creating a second identity
-- column, while these bounded columns distinguish owner intent from the
-- deployment operator that mediated it.
--
-- The owner branch rejects a NULL in every provenance column before testing
-- its shape. A bare `col ~ '...'` on a NULL column yields NULL, and a CHECK is
-- satisfied by NULL, so `FALSE OR NULL` would silently admit an owner-origin
-- row with no owner key, no mediating operator, or no acknowledgement version.
--
-- The null rejection is spelled `NOT (col IS NULL)` rather than
-- `col IS NOT NULL`: pgschema drops a named CHECK whose body contains
-- `IS NOT NULL` and still exits 0, which would leave the desired-state
-- bootstrap in `schema/schema.sql` silently unguarded while the migration path
-- stayed correct. `store::deletion::owner_provenance_contract` asserts both
-- schema sources against one case table so that divergence fails loudly.
SET LOCAL lock_timeout = '5s';

ALTER TABLE community_deletion_requests
    ADD COLUMN request_origin TEXT NOT NULL DEFAULT 'operator'
        CHECK (request_origin IN ('operator', 'owner')),
    ADD COLUMN owner_pubkey TEXT,
    ADD COLUMN mediating_operator_pubkey TEXT,
    ADD COLUMN acknowledgement_version INTEGER,
    ADD CONSTRAINT community_deletion_owner_provenance CHECK (
        (request_origin = 'operator'
            AND owner_pubkey IS NULL
            AND mediating_operator_pubkey IS NULL
            AND acknowledgement_version IS NULL)
        OR
        (request_origin = 'owner'
            AND NOT (owner_pubkey IS NULL)
            AND NOT (mediating_operator_pubkey IS NULL)
            AND NOT (acknowledgement_version IS NULL)
            AND owner_pubkey ~ '^[0-9a-f]{64}$'
            AND mediating_operator_pubkey ~ '^[0-9a-f]{64}$'
            AND acknowledgement_version BETWEEN 1 AND 32767
            AND requested_by = owner_pubkey)
    );

CREATE OR REPLACE FUNCTION prevent_community_deletion_request_retargeting()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.community_host IS DISTINCT FROM OLD.community_host
    THEN
        RAISE EXCEPTION 'community deletion target identity is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF NEW.request_origin IS DISTINCT FROM OLD.request_origin
        OR NEW.owner_pubkey IS DISTINCT FROM OLD.owner_pubkey
        OR NEW.mediating_operator_pubkey IS DISTINCT FROM OLD.mediating_operator_pubkey
        OR NEW.acknowledgement_version IS DISTINCT FROM OLD.acknowledgement_version
    THEN
        RAISE EXCEPTION 'community deletion request provenance is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.inventory_frozen_at IS NOT NULL AND (
        NEW.schema_manifest IS DISTINCT FROM OLD.schema_manifest
        OR NEW.storage_manifest IS DISTINCT FROM OLD.storage_manifest
        OR NEW.inventory_manifest IS DISTINCT FROM OLD.inventory_manifest
        OR NEW.inventory_digest IS DISTINCT FROM OLD.inventory_digest
        OR NEW.inventory_frozen_at IS DISTINCT FROM OLD.inventory_frozen_at
    ) THEN
        RAISE EXCEPTION 'frozen community deletion inventory is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF OLD.destructive_storage_frozen_at IS NOT NULL AND (
        NEW.destructive_storage_manifest IS DISTINCT FROM OLD.destructive_storage_manifest
        OR NEW.destructive_storage_frozen_at IS DISTINCT FROM OLD.destructive_storage_frozen_at
    ) THEN
        RAISE EXCEPTION 'frozen destructive storage manifest is immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END;
$$;
