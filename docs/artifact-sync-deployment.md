# Incremental artifact reads deployment

Incremental artifact reads use a 120-second overlap and database modification
timestamps. `BUZZ_ARTIFACT_INCREMENTAL_ENABLED` is off by default. Enable it only
after the schema, indexes, and every serving writer are upgraded.

## Schema and indexes

Migration `0077` adds columns with constant defaults, then changes the defaults
for future inserts. It does not rewrite existing rows or run a backfill.
Old rows remain at the epoch timestamp. An initial read includes them.
New application writers stamp every head change, grant, rejoin, removal,
redaction, and move source. Changes to channel listing also require recovery. Old application updates do not maintain all these
stamps, so mixed-version serving cannot advertise incremental correctness.

For a populated database, first build the revision lookup index outside a
transaction, before migration `0076`:

```sql
CREATE INDEX CONCURRENTLY idx_artifact_revisions_artifact
    ON public.artifact_revisions (community_id, artifact_id);
```

Apply `0076_artifact_revision_lookup.sql` in a transaction after that prebuild.
Then apply `0077_artifact_sync_timestamps.sql` in a
transaction with its lock and statement budgets. The migration is idempotent
for this prebuild step. Then build these indexes outside a transaction:

```sql
CREATE INDEX CONCURRENTLY artifact_heads_modified
    ON public.artifact_heads (community_id, modified_at, artifact_id);
CREATE INDEX CONCURRENTLY artifact_heads_channel
    ON public.artifact_heads (community_id, channel_id, artifact_id);
CREATE INDEX CONCURRENTLY channel_members_sync
    ON public.channel_members (community_id, pubkey, sync_modified_at, channel_id);
CREATE INDEX CONCURRENTLY artifact_revisions_moves
    ON public.artifact_revisions (community_id, modified_at, event_id)
    WHERE source_channel_id IS NOT NULL;
```

Migration `0078` verifies each index definition and its valid, ready, and live
flags. It refuses a populated table whose index is missing. A fresh empty
database can build the indexes under the migration's short budgets.
Migration `0076` also verifies the exact definition and all three flags.
Remove an invalid index with `DROP INDEX CONCURRENTLY` before a new build.

Relay startup migration and `buzz-admin migrate` build any of these indexes
that are missing before applying `0076` and `0078`. They use the migrations'
one-second lock and five-second statement budgets, which is enough for a small
database. Each index builds in its own transaction. A database too large for that
budget fails closed with an error that points here. The migration whose index
failed is not recorded, while earlier migrations and indexes stay committed.
Prebuild the rest concurrently as above, then retry.

After all index builds succeed, run `buzz-admin migrate` with the target
`DATABASE_URL`. It verifies and records `0076`, reapplies the idempotent `0077`,
then verifies and records `0078`. Direct SQL application does not record the
migration ledger.

## Enable and rollback

Deploy the timestamp writers to every relay instance with the gate off.
Drain the old writers. All instances must use the same configured relay
signing key, because that key authenticates the progress tokens. Then enable
`BUZZ_ARTIFACT_INCREMENTAL_ENABLED=true` on the serving fleet. NIP-11 advertises
`artifacts.incremental` only when the gate is enabled. The endpoint refuses incremental
requests while the gate is off.

Before a rollback to an old writer, disable the gate on every serving
instance. Before enabling it again, upgrade every writer and advance
`communities.artifact_policy_modified_at` for each affected active community
in a reviewed maintenance transaction under community deletion admission.
This forces clients to discover current records and revalidate saved IDs.
No local absence scan is permitted.

The 120-second stamp-to-visibility bound remains an operating assumption.
Runtime measurements are tracked separately. This PR does not authorize
a deployment or a production maintenance write.
