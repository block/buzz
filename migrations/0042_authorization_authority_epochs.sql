-- Monotonic authority epoch covering every PostgreSQL-backed V1 authority
-- reduction. The epoch is transactionally advanced by table triggers, so a
-- stale restore cannot hide a principal/key/pair tombstone, membership loss,
-- invalidation, publication transition, or audio-admission transition from
-- the independent restore witness.

CREATE TABLE authorization_authority_epochs (
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    authority_epoch BIGINT NOT NULL DEFAULT 1 CHECK (authority_epoch > 0),
    status_revision BIGINT NOT NULL DEFAULT 1 CHECK (status_revision > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id)
);

INSERT INTO authorization_authority_epochs (community_id)
SELECT id FROM communities
ON CONFLICT (community_id) DO NOTHING;

-- Dedicated current-only client projection revision. The event-author key is
-- the presentation scope; issuer/subject and lifecycle history remain solely
-- in the identity authority tables.
CREATE TABLE client_status_revisions (
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    event_author_pubkey BYTEA NOT NULL CHECK (length(event_author_pubkey) = 32),
    revision BIGINT NOT NULL CHECK (revision > 0),
    disposition TEXT NOT NULL CHECK (disposition IN ('current', 'withdrawn')),
    binding_id UUID,
    binding_version BIGINT CHECK (binding_version > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, event_author_pubkey),
    CHECK (
        (disposition = 'current' AND binding_id IS NOT NULL AND binding_version IS NOT NULL)
        OR
        (disposition = 'withdrawn' AND binding_id IS NULL AND binding_version IS NULL)
    )
);

-- Authorization state has no meaning after its owning community is removed.
-- Earlier additive migrations intentionally used restrictive foreign keys;
-- make teardown atomic now that the complete protected state set is known.
ALTER TABLE authorization_invalidation_domains
    DROP CONSTRAINT authorization_invalidation_domains_community_id_fkey,
    ADD CONSTRAINT authorization_invalidation_domains_community_id_fkey
        FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE authorization_invalidation_receipts
    DROP CONSTRAINT authorization_invalidation_receipts_community_id_fkey,
    ADD CONSTRAINT authorization_invalidation_receipts_community_id_fkey
        FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE authorization_invalidation_floors
    DROP CONSTRAINT authorization_invalidation_floors_community_id_fkey,
    ADD CONSTRAINT authorization_invalidation_floors_community_id_fkey
        FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;
ALTER TABLE authorization_operation_receipts
    DROP CONSTRAINT authorization_operation_receipts_community_id_fkey,
    ADD CONSTRAINT authorization_operation_receipts_community_id_fkey
        FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE;

CREATE FUNCTION advance_authorization_authority_epoch() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    domain_id UUID;
    invalidation_event UUID;
    invalidation_generation BIGINT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        domain_id := OLD.community_id;
    ELSE
        domain_id := NEW.community_id;
    END IF;

    -- The nested generation update below has its own table trigger. The outer
    -- protected mutation owns the epoch advancement, so the nested trigger is
    -- deliberately inert rather than double-counting one authority change.
    IF pg_trigger_depth() > 1 THEN
        RETURN NULL;
    END IF;

    -- Invalidation-domain presence is the durable marker installed during
    -- protected-domain initialization. Legacy/Off communities must retain
    -- byte-for-byte behavior and must not acquire authorization side effects.
    IF TG_TABLE_NAME <> 'authorization_invalidation_domains'
        AND NOT EXISTS (
            SELECT 1 FROM authorization_invalidation_domains
            WHERE community_id = domain_id
        )
    THEN
        RETURN NULL;
    END IF;

    -- Community teardown removes all protected state through cascading
    -- foreign keys; it cannot publish a new floor for a domain that no longer
    -- exists.
    IF TG_TABLE_NAME = 'authorization_invalidation_domains' AND TG_OP = 'DELETE' THEN
        RETURN NULL;
    END IF;

    INSERT INTO authorization_authority_epochs
        (community_id, authority_epoch, status_revision, updated_at)
    VALUES (domain_id, 2, 2, clock_timestamp())
    ON CONFLICT (community_id) DO UPDATE
    SET authority_epoch = authorization_authority_epochs.authority_epoch + 1,
        status_revision = authorization_authority_epochs.status_revision + 1,
        updated_at = clock_timestamp();

    IF TG_TABLE_NAME IN (
        'identity_bindings',
        'identity_principals',
        'identity_revoked_keys',
        'identity_retired_pairs',
        'relay_members',
        'channel_members',
        'community_bans',
        'channels',
        'users'
    ) THEN
        invalidation_event := gen_random_uuid();
        UPDATE authorization_invalidation_domains
        SET generation = generation + 1,
            updated_at = clock_timestamp()
        WHERE community_id = domain_id
        RETURNING generation INTO invalidation_generation;

        IF invalidation_generation IS NULL THEN
            RETURN NULL;
        END IF;

        INSERT INTO authorization_invalidation_receipts
            (community_id, event_id, generation, request_fingerprint)
        VALUES (
            domain_id,
            invalidation_event,
            invalidation_generation,
            digest(invalidation_event::text, 'sha256')
        );

        INSERT INTO authorization_invalidation_floors
            (community_id, selector_kind, selector_fingerprint, generation,
             sticky_deny, binding_version_floor)
        VALUES (
            domain_id,
            'domain',
            decode('a3c641c058d6498e4cc4177eb5f9cf6ba32e01c05a21f69aa85661a8044a5c78', 'hex'),
            invalidation_generation,
            FALSE,
            NULL
        )
        ON CONFLICT (community_id, selector_kind, selector_fingerprint) DO UPDATE
        SET generation = EXCLUDED.generation,
            updated_at = clock_timestamp();
    END IF;
    RETURN NULL;
END
$$;

-- Kind 30617 is live Git authorization policy. Install these triggers only
-- after the epoch function exists so fresh databases migrate in one pass.
CREATE TRIGGER git_policy_insert_authority_epoch
    AFTER INSERT ON events
    FOR EACH ROW WHEN (NEW.kind = 30617)
    EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER git_policy_update_authority_epoch
    AFTER UPDATE ON events
    FOR EACH ROW WHEN (OLD.kind = 30617 OR NEW.kind = 30617)
    EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER git_policy_delete_authority_epoch
    AFTER DELETE ON events
    FOR EACH ROW WHEN (OLD.kind = 30617)
    EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER identity_bindings_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON identity_bindings
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER identity_principals_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON identity_principals
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER identity_revoked_keys_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON identity_revoked_keys
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER identity_retired_pairs_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON identity_retired_pairs
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER relay_members_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON relay_members
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER channel_members_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON channel_members
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER community_bans_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON community_bans
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER channels_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON channels
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER users_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON users
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER authorization_invalidation_domains_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON authorization_invalidation_domains
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER git_repo_publications_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON git_repo_publications
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER media_publications_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON media_publications
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER protected_object_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON protected_object_authority
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();

CREATE TRIGGER audio_session_admissions_authority_epoch
    AFTER INSERT OR UPDATE OR DELETE ON audio_session_admissions
    FOR EACH ROW EXECUTE FUNCTION advance_authorization_authority_epoch();
