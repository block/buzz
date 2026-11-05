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
   so it can run beside that index. It lets a relay operator rename a listed
   channel, archived or not, with a name-only kind:9002, and ships the rename
   script `scripts/maintenance/rename_duplicate_listed_channel_names.sh`.
2. **Contract release.** Migration `0083_channels_listed_name_unique.sql` adds
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

A deployment that plans schema changes from the desired state in `schema/`
(see [database.md](database.md)) follows the same order: steps 1 to 4 come
first. Its tool must not build `idx_channels_listed_name` before the renames,
and the contract release must not deploy until the index is valid, because
that release no longer checks names in the relay.

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
   `operator` row in `relay_operators`), not a moderator. It must also be a
   member of each community: `POST /events` checks community membership
   before the operator role, and answers 403 `relay_membership_required` to a
   key that is not a member. Add it first:

   ```sh
   RELAY_URL=wss://<community host> buzz-admin add-member --pubkey <operator npub>
   ```

   The script stops at the first auth refusal and says so. `BUZZ_RELAY_URL`
   defaults to `wss://<community host>`. The script needs `psql` and
   `python3`. Run it with `--dry-run` first to see
   the plan. The host must match `communities.host` exactly; the script stops
   for a host that no community has.

   In each group of channels that share a name, the oldest unarchived channel
   keeps the name (when all are archived, the clients' starter channel, below,
   if it is one of them, else the oldest archived one). Each
   other channel becomes `<name>-<first 8 characters of its id>`, or
   `<name>-<id>` when another listed channel already has that name. The script
   prints each rename, then reads the plan again and exits 1 if any rename
   remains: for example, the relay refused a name that a channel took
   meanwhile. Run it again.

   The script also renames every channel named `general` or
   `welcome-everyone` when all the listed channels with that name are
   archived. Released desktop and mobile apps create those starter channels
   when no open, unarchived stream has the name, and the relay refuses that
   create while an archived channel holds the name. The apps create each
   starter at a fixed id, a UUIDv5 of the relay's HTTP origin and the name.
   The script computes those ids with `python3` from `BUZZ_RELAY_URL` (so it
   must be the URL clients use), or from `BUZZ_STARTER_ORIGIN` when set (for
   a desktop build with its own `BUZZ_RELAY_HTTP`), and prints them; check
   them against a client before the real run. When any channel row has one
   of those ids, whatever its name or state, the relay refuses the create
   there, so the script renames nothing for that starter name, and an
   archived starter at its id keeps the name over its namesakes. Those
   clients still cannot set up that starter; that needs a relay-side fix,
   planned separately.

   If a rename fails on every run because another channel holds its new name,
   rename that channel by hand with
   `buzz channels update --channel=<id> --name=<new name>`.

   The relay applies no rename in an archived community, and the script
   stops for one. Unarchive it first, then add the operator key as a member
   (`add-member` also finds only an unarchived community), run the script,
   and archive it again. Its channels show to members while it is unarchived:

   ```sh
   buzz-admin communities unarchive --host <community host> \
     --owner-pubkey <owner hex> --operator-id <you> --reason "rename duplicate channels"
   RELAY_URL=wss://<community host> buzz-admin add-member --pubkey <operator npub>
   # run the rename script for <community host>
   buzz-admin communities archive --host <community host> \
     --owner-pubkey <owner hex> --operator-id <you> --reason "rename duplicate channels"
   ```

   The relay applies no rename in a community that is being deleted either,
   and the script stops for one. Its duplicates block migration 0083 until its
   deletion completes. If the deletion is blocked, fix the recorded failure
   and resume it as
   [operator-community-deletion.md](operator-community-deletion.md)
   describes; do not deploy the contract release until it completes.
3. Confirm that no duplicate remains in any community:

   ```sql
   SELECT co.host, co.archived_at, co.deletion_state, lower(ch.name), count(*)
   FROM public.channels ch
   JOIN public.communities co ON co.id = ch.community_id
   WHERE ch.visibility = 'open' AND NOT ch.unlisted
     AND ch.channel_type IN ('stream', 'forum') AND ch.deleted_at IS NULL
   GROUP BY co.host, co.archived_at, co.deletion_state, lower(ch.name)
   HAVING count(*) > 1;
   ```

   Expect zero rows. Each row names a host to run the script for: an archived
   community, or one being deleted, needs the steps in step 2.
4. Only for a deployment that migrates out of band (auto-migration off) and
   whose `channels` table is too large for the migration's budgets: build the
   index concurrently first. Otherwise skip this step. Migration 0083 builds
   the index itself with a plain `CREATE UNIQUE INDEX`, under a 1s
   `lock_timeout` and a 5s `statement_timeout`, which takes milliseconds on a
   table of a few thousand channels. If the build cannot get its lock or
   finish within those budgets, the migration fails, records nothing, and the
   relay retries at its next start. A plain build blocks channel writes while
   it runs; a concurrent one does not. `CREATE INDEX CONCURRENTLY` cannot run
   inside a transaction block:

   ```sql
   CREATE UNIQUE INDEX CONCURRENTLY idx_channels_listed_name
       ON public.channels (community_id, lower(name))
       WHERE visibility = 'open' AND NOT unlisted
         AND channel_type IN ('stream', 'forum') AND deleted_at IS NULL;
   ```

   Then verify that PostgreSQL considers the index valid, ready, live, and an
   exact match for the definition that migration 0083 expects. Set the
   `search_path` first: without `public` on it, PostgreSQL prints the enum casts
   as `public.channel_visibility` and the text no longer matches.

   ```sql
   SET search_path = public, pg_catalog;
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
5. Apply migration 0083. It refuses to run while duplicates remain. With no
   index, it builds one; over a prebuilt index it skips its own `CREATE`.
   Then it checks the index and records the migration.
   - **Auto-migration on:** deploy the contract release. The relay applies
     0083 at startup. It does not start while a duplicate remains, so finish
     steps 2 and 3 first.
   - **Auto-migration off:** run `buzz-admin migrate` with the target
     `DATABASE_URL`, confirm that it records 0083, then deploy the contract
     release. Nothing at startup checks the index, and the contract release
     no longer checks names in the relay. Do not run the file with `psql`: that
     does not record the migration ledger, and outside a transaction both
     `SET LOCAL` timeouts do nothing.

## Recovery

A concurrent build fails if a duplicate remains or appears while it runs, and
leaves an invalid index. Migration 0083 rejects an invalid, unready, non-live,
or differently defined index. Remove only that index, outside a transaction,
run the rename script again, repeat step 3, and build again (step 4 or 5):

```sql
DROP INDEX CONCURRENTLY IF EXISTS public.idx_channels_listed_name;
```

## Rollback

Roll back from the contract release to the writer release, not further: an
older relay cannot report the index's refusal as `invalid:`. Keep the index.
Run the writer release with auto-migration off (`BUZZ_AUTO_MIGRATE=false`, or
`migrate.autoMigrate=false` in the Helm chart). With it on, the writer release
refuses a database that has migration 0083, which it does not know
(`VersionMissing`).
