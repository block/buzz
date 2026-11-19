# Community artifact activation

Community artifacts implement [NIP-AR](nips/NIP-AR.md). The operator switch
`BUZZ_COMMUNITY_ARTIFACTS_ENABLED` defaults to `false`. Enabling it requires a
restart and a verified contracted schema; setting it on the expand schema
serves channel artifacts but neither accepts nor advertises community scope.

## Expand and deploy

For an existing database, apply migration 0097. Deployments that manage
schema out of band can run the same reviewed SQL in a single transaction.
Defer any desired-state plan that changes `artifact_heads` until the guarded
contract below succeeds. The declarative schema specifies the final scoped
primary key, which old channel writers cannot use.

Migration 0097 adds constant-default scope columns, nullable channel IDs,
community revision JSON, redaction state and a bounded policy map on
`communities`. It retains the `(community_id, artifact_id)` primary key
and adds a scoped unique index.
There is no data backfill. Existing rows default to channel scope. Old channel
writers retain their conflict target and the bridge retains their advisory
lock `artifact:{tenant}:{d}`. All indexes have lock and statement budgets. If
an inline index build exceeds them, leave auto-migration disabled. Apply only
the additive column statements at the top of migration 0097 in a transaction
with the same lock and statement budgets, and commit. Then build the three
indexes with `CREATE [UNIQUE] INDEX CONCURRENTLY` outside a transaction, using
the exact names and column order in 0097. Rerun 0097 to validate their shapes
and record the migration. The additive statements are idempotent for this
procedure; a mismatched column or index still fails closed.

Deploy this audience-enforcing relay, compatible deletion executors and
operator tools to **every** backend serving the tenant, with the switch off.
Ordinary query, history, ID, search, count, moderation preview and fan-out
fences apply even while the switch is off. Community payloads are stored only
in the existing revision ledger, outside `events` and its generated search
index. They do not enter chat counters, push or hooks. The community purge deletes both scoped ledgers and clears the policy map
before retaining the community tombstone. Compatible deletion executors are
required after contract, alongside serving backends.

An old binary can continue channel operations on the expanded schema. If
rolling back before contract, disable auto-migration on the old binary: sqlx
otherwise refuses migration versions newer than the binary knows.

## Separate operator contract

This step is intentionally absent from automatic migration history. Do not run
it in a migration boot hook. An upgrade skipping the bridge deployment must
stop after expand. After verifying all backends enforce community audiences
and use scoped column inference for upserts, run in one operator session:

```sh
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 \
  -c "SET buzz.community_artifact_contract_ready = 'all-backends-enforce-community-audience'" \
  -f scripts/maintenance/community-artifact-contract.sql
```

The attestation is a human fleet check, not something a database catalog can
prove. The script verifies the expanded unique index and atomically attaches
it as the scoped primary key while removing the legacy primary key, within
5-second lock and 30-second statement budgets. The script is safe to rerun.
After it succeeds, resume desired-state reconciliation for `artifact_heads`.
The declarative schema now matches the scoped primary key. Fresh databases
bootstrapped from `schema/schema.sql` start with that key and require compatible
relay binaries, deletion executors and operator tools from the start.

Restart compatible backends with `BUZZ_COMMUNITY_ARTIFACTS_ENABLED=true` only
after the fleet check. Each process verifies the scoped primary key, absence
of the old unique key and the expanded columns before accepting writes or
advertising `artifacts.community_scope`. NIP-11 resolves the canonical tenant
UUID and advertises `max_policies: 256`. Verify signed reads and writes in both
scopes, including simultaneous identical `d` values, before allowing clients.

## Rollback floor

After contract, this audience-enforcing bridge is the minimum relay version,
even if community writes have not started. Old binaries cannot use their
legacy conflict target. Keep the contracted schema and all privacy fences.
Turning the switch off stops explicit community operations and advertisement;
it does not remove stored payloads or make them ordinary events. Do not restore
the old unique key once both scopes can contain the same `d`. Returning to an
older binary requires a separate reviewed recovery plan and is outside this
activation procedure. No live deployment is part of implementing this feature.

## Admission and exact-copy lifecycle

Community admission keeps the authenticated key and its verified owner as
restriction identities. Direct roster membership is sufficient. Owner roster
membership is a fallback only for a verified FI agent assertion or when the
current OA policy permits the verified proof. A timeout blocks new content and
redaction but permits reads and exact accepted-content retries; bans block both.

| State | Entry and exit fence |
|---|---|
| Subscribe | Claim the subscription generation under the existing lifecycle lock, retain the tenant artifact topic, and confirm Redis interest on one unbroken loss epoch before reading. |
| DB snapshot | Check admission and read current heads in one writer-backed repeatable-read transaction, keyset-paged by identity and indexed by tenant, scope and immutable type. Hold at most 256 heads per page and the advertised set bound overall. |
| Enqueue | Reserve output first. The socket credential gate, subscription generation and pending invalidation are checked before enqueue. |
| Live | A bounded typed Redis signal carries only identity, immutable type and change cause. Check admission and reload the authorized current head on the writer. A denied admission ends the sync with resync. The signal carries no payload. Current-head redaction resyncs that type; historical redaction does not retire its successor. |
| Invalidation | Existing principal/owner disconnect fan-out handles revocation. If an invalidation is missed, an idle sync can retain its set until the next relevant signal or socket revocation. The socket-level revalidator still handles archive, deletion and required-FI upgrades. Decode loss, reconnect or receiver lag resync exact copies. |
| Close | CLOSE, replacement REQ, disconnect and refusal retire the generation, cancel the task and release exactly the topic retained by its scope through the existing teardown. |

The relay writer pool defaults to 50 connections (`BUZZ_DB_POOL_SIZE`) with a
3 second acquisition timeout.
Community snapshots additionally cap themselves at `min(pool_size - 1, 8)`,
wait at most 1 second for that permit, and refuse one-connection pools. This
leaves a writer slot available when snapshots are paused on output. A
community snapshot or live head read takes one writer checkout with a 2 second
acquisition deadline and a 2 second statement timeout. The existing 30 second
snapshot budget includes paging, enqueue waits and authorization. Other writer
traffic may still saturate the pool; bounded refusal/resync preserves access
control in that case. Ordinary channel sync and ordinary event wire shapes
retain their existing paths.
