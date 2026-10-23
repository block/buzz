-- OUT-OF-BAND MAINTENANCE: do not run from relay startup migrations.
--
-- Make existing member/bot rows follow the rule that the relay now applies
-- on every write: an active `member` or `bot` row stores `bot` when its key
-- has a recorded agent owner (users.agent_owner_pubkey), and `member` when it
-- does not. Bot and member have the same permissions, so no access changes.
--
-- Preconditions:
--   1. Every relay runs a build with the write rule (PR #8494), and every
--      older relay has stopped. An older relay can still write a row that
--      breaks the rule after this script ran.
--   2. Run with psql, outside any BEGIN: psql -v ON_ERROR_STOP=1 -f <this file>
--
-- After it finishes, restart the relays one at a time. At startup, each relay
-- starts a background task that republishes each kind:39002 roster whose roles
-- now differ from the rows.
--
-- How it runs: one key per transaction. Each transaction takes the shared
-- community deletion lock, rereads the community state under that lock, and
-- skips the key unless the community is active. It then takes the key's
-- agent-owner advisory lock, the same lock that the relay's first owner
-- record and every member/bot write take, so it cannot undo a concurrent
-- owner record. It is idempotent: a converged database only reads. Rerun it
-- after an interruption.
--
-- Every repair transaction must be READ COMMITTED: under REPEATABLE READ or
-- SERIALIZABLE the community state read after the lock can come from a
-- snapshot taken before the lock, and miss a fence committed while the
-- transaction waited. The script sets READ COMMITTED for this session, and
-- the repair block refuses to run under any other isolation level.
--
-- Expected result: both validation queries at the end return 0.

SET default_transaction_isolation = 'read committed';

\echo 'Rows to change (read only):'
SELECT count(*) AS rows_to_change
FROM channel_members cm
LEFT JOIN users u
  ON u.community_id = cm.community_id AND u.pubkey = cm.pubkey
WHERE cm.removed_at IS NULL
  AND (   (cm.role = 'member' AND u.agent_owner_pubkey IS NOT NULL)
       OR (cm.role = 'bot'    AND u.agent_owner_pubkey IS NULL));

DO $$
DECLARE
    key RECORD;
    target member_role;
    active BOOLEAN;
    changed BIGINT;
    total BIGINT := 0;
BEGIN
    FOR key IN
        SELECT DISTINCT cm.community_id, cm.pubkey
        FROM channel_members cm
        LEFT JOIN users u
          ON u.community_id = cm.community_id AND u.pubkey = cm.pubkey
        WHERE cm.removed_at IS NULL
          AND (   (cm.role = 'member' AND u.agent_owner_pubkey IS NOT NULL)
               OR (cm.role = 'bot'    AND u.agent_owner_pubkey IS NULL))
        ORDER BY cm.community_id, cm.pubkey
    LOOP
        -- Same rule as lock_community_deletion_shared() in crates/buzz-db.
        IF current_setting('transaction_isolation') <> 'read committed' THEN
            RAISE EXCEPTION 'bot role repair requires read committed isolation, got %',
                current_setting('transaction_isolation');
        END IF;
        SET LOCAL lock_timeout = '5s';
        PERFORM pg_advisory_xact_lock_shared(community_deletion_lock_key(key.community_id));
        SELECT c.deletion_state = 'active' AND c.deleted_at IS NULL
          INTO active
          FROM communities c
         WHERE c.id = key.community_id;
        IF active IS NOT TRUE THEN
            COMMIT;
            CONTINUE;
        END IF;

        -- Matches agent_owner_lock_key() in crates/buzz-db.
        PERFORM pg_advisory_xact_lock(hashtextextended(
            'buzz_agent_owner:' || key.community_id::text || ':' || encode(key.pubkey, 'hex'),
            0));

        SELECT CASE WHEN EXISTS (
                   SELECT 1 FROM users u
                    WHERE u.community_id = key.community_id
                      AND u.pubkey = key.pubkey
                      AND u.agent_owner_pubkey IS NOT NULL)
               THEN 'bot'::member_role ELSE 'member'::member_role END
          INTO target;

        UPDATE channel_members
           SET role = target
         WHERE community_id = key.community_id
           AND pubkey = key.pubkey
           AND removed_at IS NULL
           AND role IN ('member', 'bot')
           AND role <> target;
        GET DIAGNOSTICS changed = ROW_COUNT;
        total := total + changed;
        COMMIT;
    END LOOP;
    RAISE NOTICE 'member/bot rows changed: %', total;
END
$$;

\echo 'Validation, expect 0: owned agents still stored as member in active communities'
SELECT count(*) AS owned_member_rows
FROM channel_members cm
JOIN users u
  ON u.community_id = cm.community_id AND u.pubkey = cm.pubkey
JOIN communities c
  ON c.id = cm.community_id AND c.deletion_state = 'active' AND c.deleted_at IS NULL
WHERE cm.removed_at IS NULL
  AND cm.role = 'member'
  AND u.agent_owner_pubkey IS NOT NULL;

\echo 'Validation, expect 0: keys with no recorded owner still stored as bot in active communities'
SELECT count(*) AS unowned_bot_rows
FROM channel_members cm
JOIN communities c
  ON c.id = cm.community_id AND c.deletion_state = 'active' AND c.deleted_at IS NULL
LEFT JOIN users u
  ON u.community_id = cm.community_id AND u.pubkey = cm.pubkey
WHERE cm.removed_at IS NULL
  AND cm.role = 'bot'
  AND u.agent_owner_pubkey IS NULL;
