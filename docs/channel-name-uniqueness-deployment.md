# Listed channel name deployment

Two listed channels in a community must not share a name, ignoring case. A
listed channel is an open, not unlisted stream or forum that is not deleted.
An archived channel still holds its name until it is deleted. Private
channels, DMs, workflow and system channels may repeat names.

The rule ships in two releases:

1. **Writer release: the first relay release that contains
   [#8782](https://github.com/block/buzz/pull/8782).** The relay refuses a
   create or rename that would give a listed channel a name another listed
   channel has, with an `invalid:` reply. It
   also reports a write that the contract index refuses with the same reply,
   so it can run beside that index. It lets a relay operator rename any
   channel with a name-only kind:9002, and ships the rename script
   `scripts/maintenance/rename_duplicate_listed_channel_names.sh`.
2. **Contract release.** Migration `0079_channels_listed_name_unique.sql` adds
   the unique index that makes the database enforce the rule:

   ```sql
   CREATE UNIQUE INDEX idx_channels_listed_name
       ON public.channels (community_id, lower(name))
       WHERE visibility = 'open' AND NOT unlisted
         AND channel_type IN ('stream', 'forum') AND deleted_at IS NULL;
   ```

   From this release the index, not a relay check, enforces the rule.

The writer release is a required stop. Do not upgrade past it to the contract
release until every step below is complete.

## Procedure

1. Deploy the writer release, and wait until no older relay pod serves. Check
   the image of every pod: a database with no duplicates does not prove that
   older relays are gone, and an older relay can still create a duplicate.
2. Rename the existing duplicates in each community. Relays keep serving.
   The script reads a plan from the database (a read-only query) and sends
   each rename to the relay as a kind:9002, signed by a relay operator. The
   relay checks the name and publishes the new kind:39000 as for any rename,
   so clients show the new name.

   ```sh
   DATABASE_URL=postgres://... BUZZ_PRIVATE_KEY=<operator key> \
     scripts/maintenance/rename_duplicate_listed_channel_names.sh <community host>
   ```

   The key must belong to a relay operator (`RELAY_OPERATOR_PUBKEYS`, or an
   `operator` row in `relay_operators`), not a moderator. `BUZZ_RELAY_URL`
   defaults to `wss://<community host>`. Run it with `--dry-run` first to see
   the plan.

   In each group of channels that share a name, the oldest unarchived channel
   keeps the name (or the oldest archived one, when all are archived). Each
   other channel becomes `<name>-<first 8 characters of its id>`, or
   `<name>-<id>` when another listed channel already has that name. The script
   prints each rename, then reads the plan again and exits 1 if any rename
   remains: for example, the relay refused a name that a channel took
   meanwhile. Run it again.

   The plan leaves out a community that is not active. A duplicate there
   blocks migration 0079 until that community's deletion completes.
3. Confirm that no duplicate remains in any community:

   ```sql
   SELECT community_id, lower(name), count(*)
   FROM public.channels
   WHERE visibility = 'open' AND NOT unlisted
     AND channel_type IN ('stream', 'forum') AND deleted_at IS NULL
   GROUP BY 1, 2
   HAVING count(*) > 1;
   ```

   Expect zero rows.
4. Build the index concurrently. Migration 0079 builds it only on an empty
   `channels` table; on a populated one it refuses to start the relay without
   the prebuilt index, because a plain build blocks channel writes.
   `CREATE INDEX CONCURRENTLY` cannot run inside a transaction block:

   ```sql
   CREATE UNIQUE INDEX CONCURRENTLY idx_channels_listed_name
       ON public.channels (community_id, lower(name))
       WHERE visibility = 'open' AND NOT unlisted
         AND channel_type IN ('stream', 'forum') AND deleted_at IS NULL;
   ```

   Then verify that PostgreSQL considers the index valid, ready, live, and an
   exact match for the definition that migration 0079 expects:

   ```sql
   SELECT i.indisvalid, i.indisready, i.indislive,
          pg_get_indexdef(i.indexrelid) AS definition
   FROM pg_index AS i
   JOIN pg_class AS c ON c.oid = i.indexrelid
   JOIN pg_namespace AS n ON n.oid = c.relnamespace
   WHERE n.nspname = 'public' AND c.relname = 'idx_channels_listed_name';
   ```

   Expect all three flags to be `true`, and this definition:

   ```text
   CREATE UNIQUE INDEX idx_channels_listed_name ON public.channels USING btree (community_id, lower((name)::text)) WHERE ((visibility = 'open'::channel_visibility) AND (NOT unlisted) AND (channel_type = ANY (ARRAY['stream'::channel_type, 'forum'::channel_type])) AND (deleted_at IS NULL))
   ```

   The writer release works with the index in place.
5. Apply migration 0079, then deploy the contract release. Migration 0079
   skips its own `CREATE`, checks the index again, and records the migration.
   It refuses to run while duplicates remain or without a valid index. With
   auto-migration on, the relay applies it at startup and does not start until
   steps 2 to 4 are complete. With auto-migration off, nothing at startup
   checks the index, and the contract release no longer checks names in the
   relay: apply 0079 out of band and confirm that it is recorded before you
   deploy.

## Recovery

A concurrent build fails if a duplicate remains or appears while it runs, and
leaves an invalid index. Migration 0079 rejects an invalid, unready, non-live,
or differently defined index. Remove only that index, outside a transaction,
run the rename script again, and repeat steps 3 and 4:

```sql
DROP INDEX CONCURRENTLY IF EXISTS public.idx_channels_listed_name;
```

## Rollback

Roll back from the contract release to the writer release, not further: an
older relay cannot report the index's refusal as `invalid:`. Keep the index.
Run the writer release with auto-migration off (`BUZZ_AUTO_MIGRATE=false`, or
`migrate.autoMigrate=false` in the Helm chart). With it on, the writer release
refuses a database that has migration 0079, which it does not know
(`VersionMissing`).
