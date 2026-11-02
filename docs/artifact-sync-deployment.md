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

Without durable certification on every serving writer, the 120-second
stamp-to-visibility bound remains an operating assumption. Runtime measurements
are tracked separately.

## Durable certification

Migration `0081` adds `artifact_sync_pending`. It is an expand migration.
It adds no trigger, function, foreign key, or backfill. Old relays ignore this
table. New relays require it before they accept relevant writes. With startup
migrations enabled, the older binary still has the SQLx `VersionMissing`
restriction described in `docs/database.md`.

Each supported writer records one pending operation in its admitted transaction.
The operation has a unique UUID. Its timestamp precedes every relevant sync
stamp in that transaction. Repeated changes in the transaction reuse the record.
Rollback removes both the changes and the record. An uncertain commit leaves
both committed or neither committed. Recovery does not require a caller retry.

Every relay starts a recovery worker, even when the incremental gate is off.
The worker reads bounded batches. It locks pending rows with `SKIP LOCKED`.
After it observes a committed record, it reads database time. If less than
120 seconds separate that time from the early timestamp, the overlap covers
visibility of the write. A clock reversal or a delay of at least 120 seconds
requires a reset. A delayed recovery can cause an unnecessary reset.

Recovery advances `artifact_policy_modified_at` only when it requires a reset.
It removes the pending rows in that same transaction. A crash before this
commit leaves the pending rows for retry. A retry after the commit finds no
work. Ordinary writes do not acquire a shared counter or policy row lock.

Existing clients use the existing `policy_changed` reset. They start discovery
again and revalidate saved IDs. The request, response, cursor, checkpoint, and
capability formats stay at version 1. No client change is required. A reset
covers the community because that is the existing policy boundary.

Full coverage requires all serving writers to use this release. The covered
paths are artifact create, update, move, delete, restore, and redaction;
channel creation; membership grants, rejoins, and removals; channel visibility,
listing, TTL, and deletion changes; DM creation; and admin member removals.
Archive state and hidden DMs do not change artifact access. Incremental reads
exclude TTL channels. Whole-community deletion stops reads and purges pending
records. Operator SQL and mixed old writers cannot provide this guarantee.
Keep the gate off during a mixed rollout. Use the existing reset procedure
before enablement or after a rollback to an old writer.

This change does not authorize deployment, gate enablement, or a production
maintenance write.
