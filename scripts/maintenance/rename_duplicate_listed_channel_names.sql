-- OUT-OF-BAND MAINTENANCE: do not run from relay startup migrations.
--
-- Gives every listed open channel a name that no other listed open channel in
-- its community has, ignoring case. A listed channel is an open, not unlisted
-- stream or forum that is not deleted; an archived one still counts. This is
-- the rule the relay enforces on create and rename (`LISTED_CHANNEL_SQL` in
-- crates/buzz-db/src/store/channel.rs). It repairs the duplicates created
-- before that rule, so the contract migration's unique index can build.
--
-- Run it only after every serving relay enforces the rule. An older relay can
-- create a new duplicate while the script runs.
--
-- In each group of channels that share a name, one channel keeps it: the
-- oldest unarchived one, or the oldest archived one when all are archived.
-- Every other channel in the group becomes "<name>-<first 8 characters of its
-- id>", or "<name>-<id>" when another listed channel already has that name.
-- The script prints a NOTICE for each rename. When both names are taken, it
-- leaves the channel unchanged with a NOTICE, and the validation below
-- reports it.
--
-- The rename writes only the channel row. Clients read the name from the
-- relay-signed kind:39000, so republish it for each renamed channel:
--
--   buzz-admin reconcile-channels --channel <id> --metadata
--
-- Each batch renames up to 100 channels in one community and commits. It
-- takes the community deletion lock first and skips a community that is not
-- active. It locks every row of the batch before it takes the relay's name
-- lock for any new name, in the order the relay takes them. After it locks a
-- row, it checks again that the row is still a listed duplicate, because a
-- relay may have changed it meanwhile. The script is idempotent: a second run
-- finds nothing to rename.
--
-- Run it outside a transaction, so that each batch can commit:
--
--   psql -v ON_ERROR_STOP=1 -f scripts/maintenance/rename_duplicate_listed_channel_names.sql

DO $$
DECLARE
    target UUID;
    skipped UUID[] := '{}';
    batch UUID[];
    duplicate UUID;
    live RECORD;
    new_name TEXT;
    renamed INT;
BEGIN
    LOOP
        SELECT ch.community_id INTO target
        FROM channels ch
        WHERE ch.visibility = 'open' AND NOT ch.unlisted
          AND ch.channel_type IN ('stream', 'forum') AND ch.deleted_at IS NULL
          AND ch.community_id <> ALL (skipped)
        GROUP BY ch.community_id, lower(ch.name)
        HAVING count(*) > 1
        LIMIT 1;
        EXIT WHEN target IS NULL;

        PERFORM set_config('lock_timeout', '5s', true);
        PERFORM pg_advisory_xact_lock_shared(community_deletion_lock_key(target));
        IF NOT EXISTS (
            SELECT 1 FROM communities
            WHERE id = target AND deletion_state = 'active' AND deleted_at IS NULL
        ) THEN
            RAISE NOTICE 'skipped community %: it is not active', target;
            skipped := skipped || target;
            COMMIT;
            CONTINUE;
        END IF;

        -- Lock every row of the batch before any name lock, in the order
        -- the relay takes them.
        SELECT array_agg(locked.id ORDER BY locked.id) INTO batch
        FROM (
            WITH ranked AS (
                SELECT id, row_number() OVER (
                    PARTITION BY lower(name)
                    ORDER BY archived_at IS NOT NULL, created_at, id
                ) AS place
                FROM channels
                WHERE community_id = target
                  AND visibility = 'open' AND NOT unlisted
                  AND channel_type IN ('stream', 'forum') AND deleted_at IS NULL
            )
            SELECT ch.id
            FROM channels ch
            JOIN ranked ON ranked.id = ch.id
            WHERE ch.community_id = target AND ranked.place > 1
            ORDER BY ch.id
            LIMIT 100
            FOR NO KEY UPDATE OF ch
        ) locked;

        renamed := 0;
        FOREACH duplicate IN ARRAY coalesce(batch, '{}')
        LOOP
            -- The ranking used the statement's snapshot. Now that the row is
            -- locked, read it again: a relay may have renamed it, made it
            -- private, or renamed the channel that kept the name.
            SELECT ch.name, ch.archived_at, ch.created_at INTO live
            FROM channels ch
            WHERE ch.community_id = target AND ch.id = duplicate
              AND ch.visibility = 'open' AND NOT ch.unlisted
              AND ch.channel_type IN ('stream', 'forum') AND ch.deleted_at IS NULL;
            CONTINUE WHEN NOT FOUND OR NOT EXISTS (
                SELECT 1 FROM channels other
                WHERE other.community_id = target AND other.id <> duplicate
                  AND lower(other.name) = lower(live.name)
                  AND other.visibility = 'open' AND NOT other.unlisted
                  AND other.channel_type IN ('stream', 'forum') AND other.deleted_at IS NULL
                  AND (other.archived_at IS NOT NULL, other.created_at, other.id)
                      < (live.archived_at IS NOT NULL, live.created_at, duplicate)
            );

            new_name := NULL;
            FOREACH new_name IN ARRAY ARRAY[
                left(live.name, 246) || '-' || left(duplicate::text, 8),
                left(live.name, 218) || '-' || duplicate::text
            ] LOOP
                PERFORM pg_advisory_xact_lock(hashtextextended(
                    'buzz_channel_name:' || target::text || ':' || lower(new_name), 0));
                EXIT WHEN NOT EXISTS (
                    SELECT 1 FROM channels other
                    WHERE other.community_id = target AND other.id <> duplicate
                      AND lower(other.name) = lower(new_name)
                      AND other.visibility = 'open' AND NOT other.unlisted
                      AND other.channel_type IN ('stream', 'forum') AND other.deleted_at IS NULL
                );
                new_name := NULL;
            END LOOP;
            IF new_name IS NULL THEN
                RAISE NOTICE 'left channel % in community % as "%": both new names are taken',
                    duplicate, target, live.name;
                CONTINUE;
            END IF;

            UPDATE channels SET name = new_name, updated_at = NOW()
            WHERE community_id = target AND id = duplicate;
            RAISE NOTICE 'renamed channel % in community % from "%" to "%"',
                duplicate, target, live.name, new_name;
            renamed := renamed + 1;
        END LOOP;

        -- A batch that found nothing to rename would find the same community
        -- again; skip it so the loop ends. The validation below reports it.
        IF renamed = 0 THEN
            skipped := skipped || target;
        END IF;
        COMMIT;
    END LOOP;
END
$$;

-- Validation. Expected result: zero rows. A row names a community the script
-- skipped (not active) or a duplicate created while it ran; rerun the script.
SELECT community_id, lower(name) AS name, count(*) AS channels
FROM channels
WHERE visibility = 'open' AND NOT unlisted
  AND channel_type IN ('stream', 'forum') AND deleted_at IS NULL
GROUP BY community_id, lower(name)
HAVING count(*) > 1;
