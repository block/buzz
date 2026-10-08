-- Permanently revoke outstanding durable invites when their issuer, recorded
-- NIP-OA owner, or currently-associated agent owner is banned. Claims and ban
-- mutations serialize on a tenant-scoped transaction advisory lock.

-- Stop concurrent ban writes while installing the fence and backfilling the
-- historical state below. An in-flight ban finishes before the lock is taken;
-- later bans wait for this migration and then run through the new triggers.
LOCK TABLE community_bans IN SHARE ROW EXCLUSIVE MODE;

ALTER TABLE relay_invites
    ADD COLUMN created_by_owner BYTEA
        CHECK (created_by_owner IS NULL OR octet_length(created_by_owner) = 32),
    ADD COLUMN revoked_at TIMESTAMPTZ;

CREATE INDEX relay_invites_issuer_live_idx
    ON relay_invites (community_id, created_by)
    WHERE revoked_at IS NULL;
CREATE INDEX relay_invites_owner_live_idx
    ON relay_invites (community_id, created_by_owner)
    WHERE revoked_at IS NULL AND created_by_owner IS NOT NULL;
CREATE INDEX users_agent_owner_pubkey_idx
    ON users (community_id, agent_owner_pubkey, pubkey)
    WHERE agent_owner_pubkey IS NOT NULL;

-- Old v2 rows did not retain an unmaterialized NIP-OA owner. Recover the
-- community-local relation when the agent's user row already records it.
UPDATE relay_invites AS invite
   SET created_by_owner = agent.agent_owner_pubkey
  FROM users AS agent
 WHERE invite.community_id = agent.community_id
   AND invite.created_by = encode(agent.pubkey, 'hex')
   AND agent.agent_owner_pubkey IS NOT NULL;

CREATE OR REPLACE FUNCTION lock_community_invite_admission(target_community UUID)
RETURNS VOID
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'buzz_invite_admission:' || target_community::text,
        0
    ));
END;
$$;

CREATE OR REPLACE FUNCTION lock_invite_admission_for_ban()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM lock_community_invite_admission(NEW.community_id);
    RETURN NEW;
END;
$$;

CREATE TRIGGER community_bans_invite_admission_lock
    BEFORE INSERT OR UPDATE ON community_bans
    FOR EACH ROW EXECUTE FUNCTION lock_invite_admission_for_ban();

CREATE OR REPLACE FUNCTION revoke_invites_for_banned_principal()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.banned AND (NEW.ban_expires_at IS NULL OR NEW.ban_expires_at > clock_timestamp()) THEN
        UPDATE relay_invites AS invite
           SET revoked_at = clock_timestamp()
         WHERE invite.community_id = NEW.community_id
           AND invite.revoked_at IS NULL
           AND invite.expires_at > clock_timestamp()
           AND (
               invite.created_by = encode(NEW.pubkey, 'hex')
               OR invite.created_by_owner = NEW.pubkey
               OR EXISTS (
                   SELECT 1
                     FROM users AS agent
                    WHERE agent.community_id = NEW.community_id
                      AND agent.agent_owner_pubkey = NEW.pubkey
                      AND invite.created_by = encode(agent.pubkey, 'hex')
               )
           );
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER community_bans_revoke_relay_invites
    AFTER INSERT OR UPDATE ON community_bans
    FOR EACH ROW EXECUTE FUNCTION revoke_invites_for_banned_principal();

CREATE OR REPLACE FUNCTION guard_relay_invite_mint()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    issuer_pubkey BYTEA;
BEGIN
    PERFORM lock_community_invite_admission(NEW.community_id);

    IF NEW.created_by ~* '^[0-9a-f]{64}$' THEN
        issuer_pubkey := decode(NEW.created_by, 'hex');
    END IF;

    IF EXISTS (
        SELECT 1
          FROM community_bans AS ban
         WHERE ban.community_id = NEW.community_id
           AND ban.banned
           AND (ban.ban_expires_at IS NULL OR ban.ban_expires_at > clock_timestamp())
           AND (
               ban.pubkey = issuer_pubkey
               OR ban.pubkey = NEW.created_by_owner
               OR ban.pubkey IN (
                   SELECT agent.agent_owner_pubkey
                     FROM users AS agent
                    WHERE agent.community_id = NEW.community_id
                      AND agent.pubkey = issuer_pubkey
                      AND agent.agent_owner_pubkey IS NOT NULL
               )
           )
    ) THEN
        RAISE EXCEPTION 'a banned principal cannot mint relay invites'
            USING ERRCODE = 'check_violation';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER relay_invites_banned_issuer_guard
    BEFORE INSERT ON relay_invites
    FOR EACH ROW EXECUTE FUNCTION guard_relay_invite_mint();

CREATE OR REPLACE FUNCTION guard_relay_invite_update()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.revoked_at IS NOT NULL AND NEW.revoked_at IS NULL THEN
        RAISE EXCEPTION 'relay invite revocation is permanent'
            USING ERRCODE = 'check_violation';
    END IF;

    IF NEW.use_count > OLD.use_count THEN
        -- Mixed-version v2 claim handlers lock the invite row before updating
        -- use_count. Taking the admission lock here preserves commit ordering;
        -- if a ban is concurrently revoking this row, PostgreSQL aborts one
        -- transaction rather than allowing a post-ban membership commit.
        PERFORM lock_community_invite_admission(NEW.community_id);
        IF NEW.revoked_at IS NOT NULL THEN
            RAISE EXCEPTION 'a revoked relay invite cannot be redeemed'
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER relay_invites_permanent_revocation
    BEFORE UPDATE OF revoked_at, use_count ON relay_invites
    FOR EACH ROW EXECUTE FUNCTION guard_relay_invite_update();

-- Backfill permanent invalidation from current restrictions and retained
-- community/deployment ban audit records. Later unban actions do not restore
-- these invitations.
WITH ban_history AS (
    SELECT community_id, pubkey, updated_at AS banned_at
      FROM community_bans
     WHERE banned
       AND (ban_expires_at IS NULL OR ban_expires_at > clock_timestamp())
    UNION ALL
    SELECT community_id, target_pubkey, created_at
      FROM moderation_actions
     WHERE action IN ('ban', 'resolve:ban')
       AND target_pubkey IS NOT NULL
    UNION ALL
    SELECT report_community_id, enforcement_target_pubkey, created_at
      FROM relay_admin_actions
     WHERE action = 'ban'
       AND step_marker IS NOT NULL
       AND enforcement_target_pubkey IS NOT NULL
)
UPDATE relay_invites AS invite
   SET revoked_at = COALESCE(
       (
           SELECT min(history.banned_at)
             FROM ban_history AS history
            WHERE history.community_id = invite.community_id
              AND invite.created_at <= history.banned_at
              AND (
                  invite.created_by = encode(history.pubkey, 'hex')
                  OR invite.created_by_owner = history.pubkey
                  OR EXISTS (
                      SELECT 1
                        FROM users AS agent
                       WHERE agent.community_id = history.community_id
                         AND agent.agent_owner_pubkey = history.pubkey
                         AND invite.created_by = encode(agent.pubkey, 'hex')
                  )
              )
       ),
       revoked_at
   )
 WHERE invite.revoked_at IS NULL;
