-- OUT-OF-BAND MAINTENANCE: do not run from relay startup migrations.
--
-- Indexes every stored kind:7 reaction under its target's effective author in
-- event_mentions, so `{"kinds":[7],"#p":[<author>]}` finds reactions whose
-- client did not tag the author. The relay writes this row at ingest; this
-- script repairs reactions stored before it did.
--
-- The effective author matches the relay's `effective_message_author`: for a
-- target signed by the relay key, its first `actor` tag, else its first `p`
-- tag, else the relay key; for any other target, its signer.
--
-- Run it only after every serving relay writes the row at ingest, with old
-- relays drained. A reaction an old relay stores after this script has run is
-- not indexed; run the script again to repair it.
--
-- It walks `reactions` rows with a source event in (community_id,
-- reaction_event_id) order, 5,000 per transaction, and commits each batch.
-- Each batch takes the shared community deletion lock for every community in
-- it, then writes only to communities that are active and not deleted. A
-- reaction whose target or source event is missing, or whose target is
-- soft-deleted, is skipped. Inserts use ON CONFLICT DO NOTHING, so a rerun,
-- or a reaction that already tags its target's author, adds nothing.
--
--   psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 \
--     -v relay_pubkey_hex=<relay public key, 64 hex characters> \
--     -f scripts/maintenance/backfill_reaction_target_mentions.sql
--
-- Each batch prints a NOTICE with the rows it inserted. The script ends with
-- the validation query: the number of reactions in active communities, with a
-- live target, that still have no mention row for the target's author. It
-- MUST print 0.

\set ON_ERROR_STOP on

-- psql does not substitute variables inside the DO body, so the relay key
-- reaches it as a session setting.
SELECT set_config('buzz_backfill.relay_pubkey_hex', lower(:'relay_pubkey_hex'), false)
WHERE :'relay_pubkey_hex' ~ '^[0-9a-fA-F]{64}$';

DO $$
DECLARE
    batch_size CONSTANT INT := 5000;
    relay_pubkey BYTEA;
    after_community UUID := '00000000-0000-0000-0000-000000000000';
    after_reaction BYTEA := ''::BYTEA;
    batch_rows INT;
    inserted INT;
    batch_number INT := 0;
    total_inserted BIGINT := 0;
BEGIN
    IF current_setting('buzz_backfill.relay_pubkey_hex', true) IS NULL
       OR current_setting('buzz_backfill.relay_pubkey_hex', true) = '' THEN
        RAISE EXCEPTION 'set relay_pubkey_hex to the relay public key as 64 hex characters';
    END IF;
    relay_pubkey := decode(current_setting('buzz_backfill.relay_pubkey_hex'), 'hex');

    LOOP
        CREATE TEMP TABLE reaction_batch ON COMMIT DROP AS
        SELECT community_id, reaction_event_id, event_id AS target_id,
               event_created_at AS target_created_at
        FROM reactions
        WHERE reaction_event_id IS NOT NULL
          AND (community_id, reaction_event_id) > (after_community, after_reaction)
        ORDER BY community_id, reaction_event_id
        LIMIT batch_size;
        GET DIAGNOSTICS batch_rows = ROW_COUNT;
        EXIT WHEN batch_rows = 0;

        SELECT community_id, reaction_event_id
        INTO after_community, after_reaction
        FROM reaction_batch
        ORDER BY community_id DESC, reaction_event_id DESC
        LIMIT 1;

        PERFORM pg_advisory_xact_lock_shared(community_deletion_lock_key(community_id))
        FROM (SELECT DISTINCT community_id FROM reaction_batch ORDER BY community_id) locked;

        -- A new statement, so the community state is read after the locks.
        INSERT INTO event_mentions
            (community_id, pubkey_hex, event_id, event_created_at, channel_id, event_kind)
        SELECT batch.community_id, author.pubkey_hex, reaction.id, reaction.created_at,
               reaction.channel_id, reaction.kind
        FROM reaction_batch batch
        JOIN communities community
          ON community.id = batch.community_id
         AND community.deletion_state = 'active'
         AND community.deleted_at IS NULL
        JOIN events reaction
          ON reaction.community_id = batch.community_id
         AND reaction.id = batch.reaction_event_id
         AND reaction.kind = 7
        JOIN events target
          ON target.community_id = batch.community_id
         AND target.id = batch.target_id
         AND target.created_at = batch.target_created_at
         AND target.deleted_at IS NULL
        CROSS JOIN LATERAL (
            SELECT CASE
                WHEN target.pubkey <> relay_pubkey THEN encode(target.pubkey, 'hex')
                WHEN tagged.actor ~ '^[0-9a-fA-F]{64}$' THEN lower(tagged.actor)
                ELSE coalesce(tagged.mention, encode(target.pubkey, 'hex'))
            END AS pubkey_hex
            FROM (SELECT
                (SELECT tag->>1
                 FROM jsonb_array_elements(target.tags) WITH ORDINALITY AS t(tag, position)
                 WHERE tag->>0 = 'actor' AND tag->>1 IS NOT NULL
                 ORDER BY position LIMIT 1) AS actor,
                (SELECT lower(tag->>1)
                 FROM jsonb_array_elements(target.tags) WITH ORDINALITY AS t(tag, position)
                 WHERE tag->>0 = 'p' AND tag->>1 ~ '^[0-9a-fA-F]{64}$'
                 ORDER BY position LIMIT 1) AS mention
            ) tagged
        ) author
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS inserted = ROW_COUNT;

        batch_number := batch_number + 1;
        total_inserted := total_inserted + inserted;
        RAISE NOTICE 'batch %: % reactions read, % mention rows inserted',
            batch_number, batch_rows, inserted;
        COMMIT;
    END LOOP;

    RAISE NOTICE 'done: % batches, % mention rows inserted', batch_number, total_inserted;
END
$$;

-- Validation: MUST print 0.
SELECT count(*) AS reactions_missing_target_author_mention
FROM reactions source
JOIN communities community
  ON community.id = source.community_id
 AND community.deletion_state = 'active'
 AND community.deleted_at IS NULL
JOIN events reaction
  ON reaction.community_id = source.community_id
 AND reaction.id = source.reaction_event_id
 AND reaction.kind = 7
JOIN events target
  ON target.community_id = source.community_id
 AND target.id = source.event_id
 AND target.created_at = source.event_created_at
 AND target.deleted_at IS NULL
CROSS JOIN LATERAL (
    SELECT CASE
        WHEN target.pubkey <> decode(current_setting('buzz_backfill.relay_pubkey_hex'), 'hex') THEN encode(target.pubkey, 'hex')
        WHEN tagged.actor ~ '^[0-9a-fA-F]{64}$' THEN lower(tagged.actor)
        ELSE coalesce(tagged.mention, encode(target.pubkey, 'hex'))
    END AS pubkey_hex
    FROM (SELECT
        (SELECT tag->>1
         FROM jsonb_array_elements(target.tags) WITH ORDINALITY AS t(tag, position)
         WHERE tag->>0 = 'actor' AND tag->>1 IS NOT NULL
         ORDER BY position LIMIT 1) AS actor,
        (SELECT lower(tag->>1)
         FROM jsonb_array_elements(target.tags) WITH ORDINALITY AS t(tag, position)
         WHERE tag->>0 = 'p' AND tag->>1 ~ '^[0-9a-fA-F]{64}$'
         ORDER BY position LIMIT 1) AS mention
    ) tagged
) author
WHERE source.reaction_event_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM event_mentions existing
      WHERE existing.community_id = source.community_id
        AND existing.pubkey_hex = author.pubkey_hex
        AND existing.event_id = reaction.id
  );
