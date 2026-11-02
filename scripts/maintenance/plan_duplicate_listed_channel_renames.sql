-- OUT-OF-BAND MAINTENANCE, read-only. rename_duplicate_listed_channel_names.sh
-- runs this query and sends each rename it lists to the relay.
--
-- Lists the renames that give every listed channel in one community a name
-- that no other listed channel there has, ignoring case. A listed channel is
-- an open, not unlisted stream or forum that is not deleted; an archived one
-- still counts. This is the rule the relay enforces on create and rename
-- (`LISTED_CHANNEL_SQL` in crates/buzz-db/src/store/channel.rs).
--
-- In each group of channels that share a name, one channel keeps it: the
-- oldest unarchived one, or when all are archived, the clients' starter
-- channel if it is one of them, else the oldest (except a starter name,
-- below).
-- Every other channel in the group gets "<name>-<first 8 characters of its
-- id>", or "<name>-<id>" when another listed channel already has that name.
-- It lists nothing for a community that is not active.
--
-- It also renames every channel named "general" or "welcome-everyone" when
-- all the listed channels with that name are archived. Released desktop and
-- mobile apps create those starter channels when no open, unarchived stream
-- has the name, and the relay refuses that create while an archived channel
-- holds the name. The rename frees the name for them. The apps create each
-- starter at a fixed id (`starter_ids`, from the rename script). When an
-- archived channel holds that id, the relay refuses the create at the id
-- whatever the name, so no rename helps: the archived starter then keeps
-- the name, and only its duplicates are renamed.
--
--   psql -X -v ON_ERROR_STOP=1 -v host=<community host> \
--     -v starter_ids='{<general id>,<welcome-everyone id>}' \
--     -f scripts/maintenance/plan_duplicate_listed_channel_renames.sql

WITH listed AS (
    SELECT ch.id, ch.name, ch.archived_at, ch.created_at,
           ch.id = ANY(:'starter_ids'::uuid[]) AS at_starter_id
    FROM channels ch
    JOIN communities co ON co.id = ch.community_id
    WHERE co.host = :'host'
      AND co.deletion_state = 'active' AND co.deleted_at IS NULL
      AND ch.visibility = 'open' AND NOT ch.unlisted
      AND ch.channel_type IN ('stream', 'forum') AND ch.deleted_at IS NULL
),
duplicates AS (
    SELECT id, name,
           left(name, 246) || '-' || left(id::text, 8) AS short_name,
           left(name, 218) || '-' || id::text AS long_name
    FROM (
        SELECT id, name,
               row_number() OVER (
                   PARTITION BY lower(name)
                   ORDER BY archived_at IS NOT NULL,
                            archived_at IS NOT NULL AND NOT at_starter_id,
                            created_at, id
               ) AS place,
               bool_and(archived_at IS NOT NULL) OVER (PARTITION BY lower(name)) AS all_archived,
               bool_or(at_starter_id) OVER (PARTITION BY lower(name)) AS starter_held
        FROM listed
    ) ranked
    WHERE place > 1
       OR (all_archived AND NOT starter_held
           AND lower(name) IN ('general', 'welcome-everyone'))
)
SELECT d.id AS channel_id, d.name,
       CASE WHEN EXISTS (
           SELECT 1 FROM listed other
           WHERE other.id <> d.id AND lower(other.name) = lower(d.short_name)
       ) THEN d.long_name ELSE d.short_name END AS new_name
FROM duplicates d
ORDER BY d.id;
