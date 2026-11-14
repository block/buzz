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
-- It walks the stored kind:7 events of each community in (created_at DESC,
-- id) order, 5,000 per transaction, and commits each batch. It reads the
-- events themselves, not `reactions` rows, because a reaction stored while
-- `set_reaction_event_id` was best-effort may have no source-event link. The
-- target is the reaction's last `e` tag holding 64 hex characters, as at
-- ingest. Each batch takes the shared community deletion lock, then writes
-- only if the community is active and not deleted. A reaction whose target is
-- missing or soft-deleted is skipped. Inserts use ON CONFLICT DO NOTHING, so a
-- rerun, or a reaction that already tags its target's author, adds nothing.
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
    current_community UUID;
    next_community UUID;
    after_created_at TIMESTAMPTZ;
    after_reaction BYTEA;
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
        SELECT id INTO next_community
        FROM communities
        WHERE current_community IS NULL OR id > current_community
        ORDER BY id
        LIMIT 1;
        EXIT WHEN next_community IS NULL;
        current_community := next_community;
        next_community := NULL;
        after_created_at := 'infinity';
        after_reaction := ''::BYTEA;

        LOOP
            -- Matches idx_events_community_kind_created (created_at DESC, id).
            CREATE TEMP TABLE reaction_batch ON COMMIT DROP AS
            SELECT id, created_at
            FROM events
            WHERE community_id = current_community
              AND kind = 7
              AND created_at <= after_created_at
              AND (created_at < after_created_at OR id > after_reaction)
            ORDER BY created_at DESC, id
            LIMIT batch_size;
            GET DIAGNOSTICS batch_rows = ROW_COUNT;
            IF batch_rows = 0 THEN
                DROP TABLE reaction_batch;
                EXIT;
            END IF;

            SELECT created_at, id INTO after_created_at, after_reaction
            FROM reaction_batch
            ORDER BY created_at, id DESC
            LIMIT 1;

            PERFORM pg_advisory_xact_lock_shared(community_deletion_lock_key(current_community));

            -- A new statement, so the community state is read after the lock.
            INSERT INTO event_mentions
                (community_id, pubkey_hex, event_id, event_created_at, channel_id, event_kind)
            SELECT reaction.community_id, author.pubkey_hex, reaction.id, reaction.created_at,
                   reaction.channel_id, reaction.kind
            FROM reaction_batch batch
            JOIN communities community
              ON community.id = current_community
             AND community.deletion_state = 'active'
             AND community.deleted_at IS NULL
            JOIN events reaction
              ON reaction.community_id = current_community
             AND reaction.id = batch.id
             AND reaction.created_at = batch.created_at
            CROSS JOIN LATERAL (
                SELECT decode(tag->>1, 'hex') AS target_id
                FROM jsonb_array_elements(reaction.tags) WITH ORDINALITY AS t(tag, position)
                WHERE tag->>0 = 'e' AND tag->>1 ~ '^[0-9a-fA-F]{64}$'
                ORDER BY position DESC LIMIT 1
            ) reference
            JOIN events target
              ON target.community_id = current_community
             AND target.id = reference.target_id
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
            RAISE NOTICE 'batch % (community %): % reactions read, % mention rows inserted',
                batch_number, current_community, batch_rows, inserted;
            COMMIT;
        END LOOP;
    END LOOP;

    RAISE NOTICE 'done: % batches, % mention rows inserted', batch_number, total_inserted;
END
$$;

-- Validation: MUST print 0. Counts every stored kind:7, linked from
-- `reactions` or not.
SELECT count(*) AS reactions_missing_target_author_mention
FROM events reaction
JOIN communities community
  ON community.id = reaction.community_id
 AND community.deletion_state = 'active'
 AND community.deleted_at IS NULL
CROSS JOIN LATERAL (
    SELECT decode(tag->>1, 'hex') AS target_id
    FROM jsonb_array_elements(reaction.tags) WITH ORDINALITY AS t(tag, position)
    WHERE tag->>0 = 'e' AND tag->>1 ~ '^[0-9a-fA-F]{64}$'
    ORDER BY position DESC LIMIT 1
) reference
JOIN events target
  ON target.community_id = reaction.community_id
 AND target.id = reference.target_id
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
WHERE reaction.kind = 7
  AND NOT EXISTS (
      SELECT 1 FROM event_mentions existing
      WHERE existing.community_id = reaction.community_id
        AND existing.pubkey_hex = author.pubkey_hex
        AND existing.event_id = reaction.id
  );
