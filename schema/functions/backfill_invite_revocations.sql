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
