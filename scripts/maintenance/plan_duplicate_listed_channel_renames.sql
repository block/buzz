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
-- oldest unarchived one, or the oldest archived one when all are archived.
-- Every other channel in the group gets "<name>-<first 8 characters of its
-- id>", or "<name>-<id>" when another listed channel already has that name.
-- It lists nothing for a community that is not active.
--
--   psql -X -v ON_ERROR_STOP=1 -v host=<community host> -f scripts/maintenance/plan_duplicate_listed_channel_renames.sql

WITH listed AS (
    SELECT ch.id, ch.name, ch.archived_at, ch.created_at
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
        SELECT id, name, row_number() OVER (
            PARTITION BY lower(name)
            ORDER BY archived_at IS NOT NULL, created_at, id
        ) AS place
        FROM listed
    ) ranked
    WHERE place > 1
)
SELECT d.id AS channel_id, d.name,
       CASE WHEN EXISTS (
           SELECT 1 FROM listed other
           WHERE other.id <> d.id AND lower(other.name) = lower(d.short_name)
       ) THEN d.long_name ELSE d.short_name END AS new_name
FROM duplicates d
ORDER BY d.id;
