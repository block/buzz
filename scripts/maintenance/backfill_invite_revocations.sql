-- This companion must be separately approved before core deployment and run
-- in that same controlled rollout, after schema migration 0069 and draining
-- every old writer. Prevent invite mint/claim, ban lifts, and NEW timeouts/mutes
-- throughout drain/repair. Without controls covering all writer paths, remove
-- public traffic and stop all relay/admin/background/direct writers; run only
-- this operator connection. Reopen only on new code after zero remaining and
-- an idempotent rerun. Review fenced communities before reopening them.
--
-- Each community batch commits independently, takes the same lock order as the
-- application writers, and skips a community unless it is active and has no
-- deletion timestamp. For active communities it records historical revocation
-- before clearing legacy_admission on every invite row. Skipped communities
-- retain the marker and remain fail-closed until a later run. The script is
-- idempotent; rerun it after a failure.
\set ON_ERROR_STOP on
SET lock_timeout = '5s';
SET statement_timeout = '2min';

DO $$
DECLARE
    last_community UUID;
    target_community UUID;
    community_active BOOLEAN;
    affected BIGINT;
BEGIN
    LOOP
        SELECT id INTO target_community
        FROM communities
        WHERE last_community IS NULL OR id > last_community
        ORDER BY id
        LIMIT 1;
        EXIT WHEN NOT FOUND;
        last_community := target_community;

        LOOP
            PERFORM pg_advisory_xact_lock_shared(
                community_deletion_lock_key(target_community)
            );
            PERFORM pg_advisory_xact_lock(
                hashtextextended('invite-admission:' || target_community::text, 0)
            );
            SELECT deletion_state = 'active' AND deleted_at IS NULL
            INTO community_active
            FROM communities
            WHERE id = target_community;

            IF community_active IS DISTINCT FROM TRUE THEN
                COMMIT;
                EXIT;
            END IF;

            WITH candidates AS (
                SELECT invite.id,
                       (
                           (invite.legacy_admission AND (
                               issuer.pubkey IS NULL
                               OR restriction_state.has_retained_restriction
                           ))
                           OR restriction_state.has_active_ban
                       ) AS should_revoke
                FROM relay_invites AS invite
                CROSS JOIN LATERAL (
                    SELECT CASE
                        WHEN octet_length(invite.created_by) = 64
                             AND invite.created_by !~ '[^0-9a-f]'
                        THEN decode(invite.created_by, 'hex')
                    END AS pubkey
                ) AS issuer
                CROSS JOIN LATERAL (
                    SELECT
                        EXISTS (
                            SELECT 1
                            FROM community_bans AS restriction
                            WHERE restriction.community_id = invite.community_id
                              AND (
                                  restriction.pubkey = issuer.pubkey
                                  OR restriction.pubkey = (
                                      SELECT user_row.agent_owner_pubkey
                                      FROM users AS user_row
                                      WHERE user_row.community_id = invite.community_id
                                        AND user_row.pubkey = issuer.pubkey
                                  )
                              )
                        ) AS has_retained_restriction,
                        EXISTS (
                            SELECT 1
                            FROM community_bans AS restriction
                            WHERE restriction.community_id = invite.community_id
                              AND (
                                  restriction.pubkey = issuer.pubkey
                                  OR restriction.pubkey = (
                                      SELECT user_row.agent_owner_pubkey
                                      FROM users AS user_row
                                      WHERE user_row.community_id = invite.community_id
                                        AND user_row.pubkey = issuer.pubkey
                                  )
                              )
                              AND restriction.banned
                              AND (restriction.ban_expires_at IS NULL
                                   OR restriction.ban_expires_at > clock_timestamp())
                        ) AS has_active_ban
                ) AS restriction_state
                WHERE invite.community_id = target_community
                  AND (
                    invite.legacy_admission
                    OR (invite.revoked_at IS NULL AND restriction_state.has_active_ban)
                  )
                ORDER BY invite.id
                LIMIT 500
                FOR UPDATE OF invite
            )
            UPDATE relay_invites AS invite
            SET revoked_at = CASE
                    WHEN candidates.should_revoke AND invite.revoked_at IS NULL
                    THEN clock_timestamp()
                    ELSE invite.revoked_at
                END,
                legacy_admission = false
            FROM candidates
            WHERE invite.community_id = target_community
              AND invite.id = candidates.id;
            GET DIAGNOSTICS affected = ROW_COUNT;

            COMMIT;
            EXIT WHEN affected = 0;
        END LOOP;
    END LOOP;
END
$$;

-- Expected result: 0. Every legacy marker in an active community has been
-- examined and cleared; every currently active ban has durably revoked its
-- invite. Markers in fenced communities remain for a later repair run.
SELECT count(*) AS unrepaired_invites
FROM relay_invites AS invite
JOIN communities AS community ON community.id = invite.community_id
CROSS JOIN LATERAL (
    SELECT CASE
        WHEN octet_length(invite.created_by) = 64
             AND invite.created_by !~ '[^0-9a-f]'
        THEN decode(invite.created_by, 'hex')
    END AS pubkey
) AS issuer
WHERE community.deletion_state = 'active'
  AND community.deleted_at IS NULL
  AND (
    invite.legacy_admission
    OR (
      invite.revoked_at IS NULL
      AND EXISTS (
        SELECT 1
        FROM community_bans AS restriction
        WHERE restriction.community_id = invite.community_id
          AND (
            restriction.pubkey = issuer.pubkey
            OR restriction.pubkey = (
                SELECT user_row.agent_owner_pubkey
                FROM users AS user_row
                WHERE user_row.community_id = invite.community_id
                  AND user_row.pubkey = issuer.pubkey
            )
          )
          AND restriction.banned
          AND (restriction.ban_expires_at IS NULL
               OR restriction.ban_expires_at > clock_timestamp())
      )
    )
  );
