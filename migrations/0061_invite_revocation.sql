ALTER TABLE relay_invites ADD COLUMN revoked_at TIMESTAMPTZ;

-- Serialize invitation admission with restriction and ownership changes per tenant.
CREATE FUNCTION invite_admission_lock(community UUID) RETURNS VOID
LANGUAGE sql AS $$
    SELECT pg_advisory_xact_lock(hashtextextended('invite-admission:' || community::text, 0));
$$;

CREATE FUNCTION lock_invite_restrictions() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM invite_admission_lock(NEW.community_id);
    RETURN NEW;
END
$$;

-- Revocation is durable: neither unban nor expiry restores an issued bearer.
CREATE FUNCTION revoke_banned_issuer_invites() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.banned AND (NEW.ban_expires_at IS NULL OR NEW.ban_expires_at > clock_timestamp()) THEN
        UPDATE relay_invites SET revoked_at = clock_timestamp()
        WHERE community_id = NEW.community_id AND revoked_at IS NULL
          AND (created_by = encode(NEW.pubkey, 'hex') OR created_by IN (
              SELECT encode(pubkey, 'hex') FROM users
              WHERE community_id = NEW.community_id AND agent_owner_pubkey = NEW.pubkey
          ));
    END IF;
    RETURN NEW;
END
$$;

-- Learning a verified owner must not revive invites issued by its restricted agent.
CREATE FUNCTION revoke_restricted_agent_invites() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.agent_owner_pubkey IS NOT NULL AND EXISTS (
        SELECT 1 FROM community_bans WHERE community_id = NEW.community_id
          AND pubkey = NEW.agent_owner_pubkey AND banned
          AND (ban_expires_at IS NULL OR ban_expires_at > clock_timestamp())
    ) THEN
        UPDATE relay_invites SET revoked_at = clock_timestamp()
        WHERE community_id = NEW.community_id AND created_by = encode(NEW.pubkey, 'hex')
          AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER invite_restriction_lock BEFORE INSERT OR UPDATE ON community_bans
FOR EACH ROW EXECUTE FUNCTION lock_invite_restrictions();
CREATE TRIGGER invite_issuer_revocation AFTER INSERT OR UPDATE ON community_bans
FOR EACH ROW EXECUTE FUNCTION revoke_banned_issuer_invites();

CREATE TRIGGER invite_owner_lock BEFORE INSERT OR UPDATE OF agent_owner_pubkey ON users
FOR EACH ROW EXECUTE FUNCTION lock_invite_restrictions();
CREATE TRIGGER invite_owner_revocation AFTER INSERT OR UPDATE OF agent_owner_pubkey ON users
FOR EACH ROW EXECUTE FUNCTION revoke_restricted_agent_invites();

-- Include active bans and recorded prior bans, so lifting a ban before this
-- migration cannot restore an invite issued before that ban. Scope every owner
-- and audit lookup to the invitation's community.
-- Schema migrations hold the exclusive schema-destruction lock. Retain the
-- universal tenant fence too: acquire its shared lock, then read the current
-- generation before applying this narrowly scoped historical security repair.
-- A quiescing tenant may return to active, so skipping it could revive bearers.
CREATE FUNCTION backfill_invite_revocations() RETURNS BIGINT
LANGUAGE plpgsql AS $$
DECLARE
    target RECORD;
    affected BIGINT;
    total BIGINT := 0;
    current_generation BIGINT;
    prior_community TEXT := current_setting('buzz.deletion_executor_community', true);
    prior_generation TEXT := current_setting('buzz.deletion_fence_generation', true);
BEGIN
    FOR target IN SELECT id FROM communities
                  WHERE EXISTS (SELECT 1 FROM relay_invites WHERE community_id = communities.id)
                  ORDER BY id
    LOOP
        PERFORM pg_advisory_xact_lock_shared(community_deletion_lock_key(target.id));
        SELECT deletion_fence_generation INTO STRICT current_generation
        FROM communities WHERE id = target.id;
        PERFORM set_config('buzz.deletion_executor_community', target.id::text, true);
        PERFORM set_config('buzz.deletion_fence_generation', current_generation::text, true);
        UPDATE relay_invites i SET revoked_at = clock_timestamp()
        WHERE i.community_id = target.id AND i.revoked_at IS NULL AND EXISTS (
            SELECT 1 FROM (
                SELECT CASE WHEN i.created_by ~ '^[0-9a-f]{64}$'
                            THEN decode(i.created_by, 'hex') END AS pubkey
                UNION ALL
                SELECT agent_owner_pubkey FROM users
                WHERE community_id = i.community_id AND encode(pubkey, 'hex') = i.created_by
                  AND agent_owner_pubkey IS NOT NULL
            ) principal
            WHERE EXISTS (
                SELECT 1 FROM community_bans b WHERE b.community_id = i.community_id
                  AND b.pubkey = principal.pubkey AND b.banned
                  AND (b.ban_expires_at IS NULL OR b.ban_expires_at > clock_timestamp())
            ) OR EXISTS (
                SELECT 1 FROM moderation_actions a WHERE a.community_id = i.community_id
                  AND a.target_pubkey = principal.pubkey AND a.action IN ('ban', 'resolve:ban')
                  AND a.created_at >= i.created_at
            ) OR EXISTS (
                SELECT 1 FROM relay_admin_actions a WHERE a.report_community_id = i.community_id
                  AND a.enforcement_target_pubkey = principal.pubkey AND a.action = 'ban'
                  AND (a.step_marker = 'mutation_committed' OR a.state = 'succeeded')
                  AND a.updated_at >= i.created_at
            )
        )
        ;
        GET DIAGNOSTICS affected = ROW_COUNT;
        total := total + affected;
        PERFORM set_config('buzz.deletion_executor_community', COALESCE(prior_community, ''), true);
        PERFORM set_config('buzz.deletion_fence_generation', COALESCE(prior_generation, ''), true);
    END LOOP;
    RETURN total;
END
$$;

SELECT backfill_invite_revocations();
